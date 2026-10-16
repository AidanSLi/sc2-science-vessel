//! Watches the game's own Client API so the app can react the moment a game starts.
//!
//! ## Why this lives in Rust and not in the frontend's poll loop
//!
//! The app is resident and its window is **hidden between games**, and hiding a WebKitGTK
//! widget unmaps it — which is precisely what makes WebKit suspend or unload a page's timers.
//! A popup whose entire job is to appear the instant a game starts therefore cannot depend on
//! a hidden webview's `setInterval` still firing. A blocking read on a background thread has no
//! such dependency, so the one behaviour that must survive being hidden is implemented on the
//! side that stays awake.
//!
//! ## The endpoint
//!
//! The game's own Client API on loopback (ADR 0004), which needs no network and no game
//! cooperation beyond what Blizzard already ships:
//!
//! ```text
//! GET http://127.0.0.1:6119/game
//! {"isReplay":false,"displayTime":132.0,"players":[...]}
//! ```
//!
//! `isReplay: false` means a real game is being played (as opposed to a replay being watched),
//! and that flag flipping from false to true is the game-start edge this module acts on.
//!
//! ## Why the HTTP request is hand-rolled
//!
//! The endpoint is a fixed, local, trusted server that answers with a few hundred bytes. A
//! blocking `TcpStream` request is a dozen lines and no new dependency, where adding an HTTP
//! client would pull a second async runtime into an app that already has one, to make a request
//! against `127.0.0.1`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::notify;
use crate::settings::Settings;

/// The game's Client API port. Matches `CLIENT_API_PORT` in `src/lib/client-api.ts`.
pub const CLIENT_API_PORT: u16 = 6119;

/// How often the client is polled.
///
/// One second, for the same reason the frontend polls at one second: a game start must be
/// noticed promptly, and a loopback request per second costs nothing.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Budget for one whole request. The server is local, so anything slower than this is a
/// stalled socket rather than a slow server, and holding the loop up would delay the next
/// poll past the point where the reading is useful.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(800);

/// The part of the Client API's `/game` response this module needs.
///
/// Deliberately not the whole payload: races, names and results are the frontend's business,
/// and re-parsing them here would mean two places to update when the game changes its output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockReading {
    /// Whether a replay is being watched rather than a game being played.
    pub is_replay: bool,
    /// The in-game clock in seconds, or `None` when the client did not report one.
    ///
    /// `None` rather than `0` because a game at zero seconds and a client that omitted the
    /// field are different states, and collapsing them would treat a missing reading as a
    /// game that had just started.
    pub display_time: Option<f64>,
}

/// Parses a `/game` response body.
///
/// Separated from the socket so the interpretation can be tested against captured payloads
/// without a game running — which is the only way this can be tested at all, since the game
/// cannot be started from inside a test.
///
/// @param body The raw JSON body from the Client API.
/// @returns The reading, or `None` when the body is not a JSON object.
pub fn parse_clock(body: &str) -> Option<ClockReading> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    Some(ClockReading {
        // Absent is treated as "not a replay" rather than as an error: the flag is only ever
        // omitted before a game loads, and the callers act on `true` in-game.
        is_replay: object.get("isReplay").and_then(|v| v.as_bool()).unwrap_or(false),
        display_time: object.get("displayTime").and_then(|v| v.as_f64()),
    })
}

/// Whether a reading means "a game is being played right now".
///
/// A replay is explicitly excluded: the player is watching rather than playing, and the whole
/// point of the popup is to help someone who is about to play.
///
/// @param reading The parsed client reading.
/// @returns `true` when a real game is live.
pub fn is_playing(reading: ClockReading) -> bool {
    !reading.is_replay
}

/// Fetches and parses `/game` in one blocking call.
///
/// Every failure is `None`: with no game running the port is simply closed, which is the
/// normal case for most of the app's life rather than an error worth reporting.
///
/// @returns The reading, or `None` when the client could not be read.
fn read_client() -> Option<ClockReading> {
    let body = get_loopback("/game")?;
    parse_clock(&body)
}

/// Performs a blocking HTTP/1.0 GET against the Client API.
///
/// HTTP/1.0 is requested on purpose: it makes the server close the connection when the body
/// ends, so the response can be read to EOF with no chunked-transfer decoding and no
/// `Content-Length` arithmetic.
///
/// @param path The request path, e.g. `/game`.
/// @returns The response body, or `None` if anything went wrong.
fn get_loopback(path: &str) -> Option<String> {
    let address = ("127.0.0.1", CLIENT_API_PORT)
        .to_socket_addrs()
        .ok()?
        .next()?;
    let mut stream = TcpStream::connect_timeout(&address, REQUEST_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT)).ok()?;

    let request = format!(
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{CLIENT_API_PORT}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;

    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;

    // Split the status line and headers from the body. A response with no body still separates
    // them, so a missing separator means the response was not HTTP at all.
    let (_, body) = response.split_once("\r\n\r\n")?;
    Some(body.to_string())
}

/// Tells the player how to open the overlay, once, when a game begins.
///
/// The shortcut is read from settings rather than passed in, so the popup names the key that
/// is actually registered: `register_overlay_shortcut` stores a substitute when the requested
/// key was taken, and telling the player to press a dead combination is worse than saying
/// nothing at all.
///
/// @param app The running app handle, for reading settings.
fn announce_game_start(app: &tauri::AppHandle) {
    let settings = Settings::load(app);
    notify::show_game_start_popup(&settings.overlay_shortcut);
}

/// Watches for a game starting and announces it.
///
/// Acts on the **rising edge** only. The client reports a live game once a second for the whole
/// match, so reacting to the state rather than to the transition would open a popup every
/// second; the previous reading is what turns "a game is running" into "a game has started".
///
/// A game already in progress when the app launches counts as a rising edge by construction —
/// the previous reading starts as `false` — which is what the player wants if they started the
/// app mid-match.
///
/// This thread runs for the life of the process and is never joined. There is exactly one of
/// it, created once in `run`, so it needs no guard against duplication.
///
/// @param app The running app handle.
pub fn watch_for_game_start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut was_playing = false;
        loop {
            let reading = read_client();
            let playing = reading.is_some_and(is_playing);

            if playing && !was_playing {
                announce_game_start(&app);
            }
            was_playing = playing;

            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_game_is_recognised_from_the_clients_own_shape() {
        // Captured verbatim from a running AI game on the reference machine.
        let body = r#"{"isReplay":false,"displayTime":132.0,"players":[
			{"id":1,"name":"flashandfoul","type":"user","race":"Prot","result":"Undecided"},
			{"id":2,"name":"A.I. 1 (Very Easy)","type":"computer","race":"random","result":"Undecided"}]}"#;
        let reading = parse_clock(body).expect("the payload should parse");
        assert!(!reading.is_replay);
        assert_eq!(reading.display_time, Some(132.0));
        assert!(is_playing(reading));
    }

    #[test]
    fn a_replay_is_not_a_game_being_played() {
        let reading = parse_clock(r#"{"isReplay":true,"displayTime":40.0}"#).unwrap();
        assert!(
            !is_playing(reading),
            "watching a replay must not trigger the hint to press the overlay key"
        );
    }

    #[test]
    fn a_missing_clock_is_not_a_clock_at_zero() {
        // The distinction matters: zero is a real reading at the start of a game, so treating
        // an absent field as zero would make a broken response look like a game just starting.
        let reading = parse_clock(r#"{"isReplay":false}"#).unwrap();
        assert_eq!(reading.display_time, None);
    }

    #[test]
    fn a_missing_is_replay_flag_does_not_count_as_a_replay() {
        let reading = parse_clock(r#"{"displayTime":12.5}"#).unwrap();
        assert!(!reading.is_replay);
    }

    #[test]
    fn a_body_that_is_not_json_is_not_a_reading() {
        assert!(parse_clock("<html>404</html>").is_none());
        assert!(parse_clock("").is_none());
        assert!(
            parse_clock("[1,2,3]").is_none(),
            "a JSON array is not the client's payload shape"
        );
    }

    #[test]
    fn the_request_is_targeted_at_the_documented_port() {
        assert_eq!(CLIENT_API_PORT, 6119);
    }
}

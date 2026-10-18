//! Watches the game's own Client API so the app can react while its window is hidden.
//!
//! Two behaviours need this, and they need it for the same reason:
//!
//! - **the hint popup** that names the overlay key when a game starts, and
//! - **the mode the window comes up in when the player presses the hotkey** — see
//!   [`hotkey_mode`], which is the rule behind "show the analysis window when the game is
//!   running but no match is on, and the guide only during a match".
//!
//! ## Why this lives in Rust and not in the frontend's poll loop
//!
//! The app is resident and its window is **hidden between games**, and hiding a WebKitGTK
//! widget unmaps it — which is precisely what makes WebKit suspend or unload a page's timers.
//! A popup whose entire job is to appear the instant a game starts therefore cannot depend on
//! a hidden webview's `setInterval` still firing, and a mode decided at a hotkey press cannot
//! ask a page that is not running. A blocking read on a background thread has no such
//! dependency, so the behaviours that must survive being hidden are implemented on the side
//! that stays awake.
//!
//! ## The endpoint
//!
//! The game's own Client API on loopback (ADR 0004), which needs no network and no game
//! cooperation beyond what Blizzard already ships:
//!
//! ```text
//! GET http://127.0.0.1:6119/game
//! {"isReplay":false,"displayTime":132.0,"players":[...]}
//! GET http://127.0.0.1:6119/ui
//! {"activeScreens":[]}
//! ```
//!
//! `isReplay: false` means a real game is being played (as opposed to a replay being watched),
//! and that flag flipping from false to true is the game-start edge this module acts on.
//! `activeScreens` says where the player *is*: an empty list is the in-game reading, and any
//! name at all means a screen is open. That second reading is what [`hotkey_mode`] decides on
//! (ADR 0010, ADR 0016).
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
use crate::WindowMode;

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

/// Narrows a `/ui` response body to the list of screens the client has open.
///
/// Deliberately strict where the frontend is lenient: a body that is not a JSON object, or that
/// has no `activeScreens` array, is `None` ("could not tell") rather than an empty list. Empty
/// has a meaning here — no screen is open, i.e. a match is being played — so a malformed
/// response must not be allowed to impersonate the in-game reading and put a click-through
/// guide over the player's menus.
///
/// @param body The raw JSON body from the Client API's `/ui` endpoint.
/// @returns The active screen names, or `None` when the body does not report them.
pub fn parse_active_screens(body: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    let screens = object.get("activeScreens")?.as_array()?;
    Some(
        screens
            .iter()
            .filter_map(|screen| screen.as_str().map(str::to_owned))
            .collect(),
    )
}

/// Fetches and parses `/ui` in one blocking call.
///
/// @returns The active screen names, or `None` when the client could not be read.
fn read_screens() -> Option<Vec<String>> {
    let body = get_loopback("/ui")?;
    parse_active_screens(&body)
}

/// Decides which mode the window should be in when the player asks to see it.
///
/// This is the rule the user asked for by name: the hotkey must bring up the **ordinary analysis
/// window while the game is running but no match is on**, and the build-order guide only while a
/// match is actually being played. "In the game" is the SC2 process, which cannot be asked over
/// the client API at all; "in a match" is an empty `/ui` screen list, exactly as ADR 0010
/// established for the automatic switch.
///
/// The screen names are deliberately not enumerated. Every non-empty list — menus, the loading
/// screen, the score screen — means the same thing for this decision (the player is not playing,
/// so a guide is useless and a clickable window is better), so naming individual screens would
/// only create a second copy of `game-mode.ts`'s vocabulary to drift out of step. A screen this
/// app has never seen therefore degrades to the analysis window, which is the harmless answer.
///
/// `None` means "leave the mode alone" rather than "analysis", mirroring `decideMode`'s `null`:
/// the one case that reaches it is a client that answered `/game` but not `/ui`, and mid-match
/// that stored mode is the guide. Guessing "analysis" from an unreadable `/ui` would put the
/// analysis pages over a running game, which is the one outcome this rule exists to prevent.
///
/// @param clock The `/game` reading, or `None` when the client API did not answer.
/// @param screens The `/ui` reading, or `None` when it could not be read.
/// @returns The mode to reveal, or `None` to keep the current one.
pub fn hotkey_mode(clock: Option<ClockReading>, screens: Option<Vec<String>>) -> Option<WindowMode> {
    let Some(clock) = clock else {
        // No answer at all. The client API is served by the game itself and is up for the whole
        // of a match — the guide's clock depends on it — so silence means no match is on: a
        // closed game, or the client API switched off. That is precisely when the player wants
        // the ordinary window, and it is also what stops a stale overlay mode (left behind by a
        // match that ended while the window was hidden) from coming back as a 380-pixel
        // click-through panel over the desktop.
        return Some(WindowMode::Analysis);
    };
    if clock.is_replay {
        // A replay's screen list is the same empty reading as a live match, and the player is
        // watching rather than playing, so its controls must be clickable (ADR 0010).
        return Some(WindowMode::Analysis);
    }
    let screens = screens?;
    if screens.is_empty() {
        Some(WindowMode::Overlay)
    } else {
        Some(WindowMode::Analysis)
    }
}

/// Reads both client endpoints and applies [`hotkey_mode`].
///
/// Blocking, and called from the hotkey's thread. That is acceptable because both requests go to
/// a local socket and are budgeted at [`REQUEST_TIMEOUT`] each; the alternative — deciding the
/// mode on a background thread and revealing the window later — would make the hotkey feel
/// unreliable in a way it is not.
///
/// `/ui` is only asked when `/game` answered: with the client API down there is nothing to ask,
/// and spending a second connect timeout to learn that would delay the window for no reason.
///
/// @returns The mode to reveal, or `None` when only the player's last mode is trustworthy.
pub fn hotkey_mode_now() -> Option<WindowMode> {
    let clock = read_client();
    let screens = if clock.is_some() {
        read_screens()
    } else {
        None
    };
    hotkey_mode(clock, screens)
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

    /// The in-game reading, captured on this machine: an empty `activeScreens` list is the
    /// information, not the absence of it.
    #[test]
    fn an_empty_screen_list_is_the_in_game_reading() {
        assert_eq!(
            parse_active_screens(r#"{"activeScreens":[]}"#),
            Some(Vec::new())
        );
    }

    /// The menu and loading readings, captured on this machine. Both must survive as non-empty
    /// lists; which names they are is deliberately not something the decision looks at.
    #[test]
    fn a_named_screen_survives_as_a_non_empty_list() {
        let menus = parse_active_screens(
            r#"{"activeScreens":["ScreenBackgroundSC2/ScreenBackgroundSC2","ScreenNavigationSC2/ScreenNavigationSC2"]}"#,
        )
        .expect("the payload should parse");
        assert_eq!(menus.len(), 2);
        let loading = parse_active_screens(r#"{"activeScreens":["ScreenLoading/ScreenLoading"]}"#)
            .expect("the payload should parse");
        assert_eq!(loading, vec!["ScreenLoading/ScreenLoading".to_string()]);
    }

    /// A response that does not report the screens is "could not tell", never "no screens":
    /// an empty list is the in-game reading, so a malformed body must not impersonate it.
    #[test]
    fn a_body_without_a_screen_list_is_nothing_not_empty() {
        assert_eq!(parse_active_screens("{}"), None);
        assert_eq!(parse_active_screens(r#"{"activeScreens":null}"#), None);
        assert_eq!(parse_active_screens(r#"{"activeScreens":"ScreenScore"}"#), None);
        assert_eq!(parse_active_screens("<html>404</html>"), None);
        assert_eq!(parse_active_screens(""), None);
    }

    /// The live reading a hotkey press sees mid-match: the guide, because the player is playing.
    #[test]
    fn a_hotkey_press_during_a_match_brings_up_the_guide() {
        let clock = parse_clock(r#"{"isReplay":false,"displayTime":132.0}"#);
        assert_eq!(hotkey_mode(clock, Some(Vec::new())), Some(WindowMode::Overlay));
    }

    /// The whole point of the change: the game is running, the player is in its menus, so the
    /// hotkey must bring up the ordinary window rather than the click-through guide.
    #[test]
    fn a_hotkey_press_in_the_menus_brings_up_the_analysis_window() {
        let clock = parse_clock(r#"{"isReplay":false,"displayTime":0}"#);
        let menus = vec!["ScreenNavigationSC2/ScreenNavigationSC2".to_string()];
        assert_eq!(hotkey_mode(clock, Some(menus)), Some(WindowMode::Analysis));
    }

    /// The score screen is a non-empty list like any other menu, and a replay watching is not a
    /// match: both are the player's own clickable window.
    #[test]
    fn a_score_screen_or_a_replay_is_the_analysis_window() {
        let clock = parse_clock(r#"{"isReplay":false,"displayTime":900.0}"#);
        let score = vec!["ScreenScore/ScreenScore".to_string()];
        assert_eq!(hotkey_mode(clock, Some(score)), Some(WindowMode::Analysis));

        let replay = parse_clock(r#"{"isReplay":true,"displayTime":40.0}"#);
        assert_eq!(hotkey_mode(replay, Some(Vec::new())), Some(WindowMode::Analysis));
    }

    /// With no client API there is no match to be in, and the answer is the ordinary window —
    /// including the case that motivated it, a stale overlay mode left behind by a game that
    /// ended while the window was hidden.
    #[test]
    fn no_client_api_means_the_analysis_window() {
        assert_eq!(hotkey_mode(None, None), Some(WindowMode::Analysis));
    }

    /// A client that answered `/game` but not `/ui` is the one case where nothing is chosen: the
    /// stored mode is the frontend's last decision, which mid-match is the guide.
    #[test]
    fn an_unreadable_screen_list_leaves_the_mode_alone() {
        let clock = parse_clock(r#"{"isReplay":false,"displayTime":132.0}"#);
        assert_eq!(hotkey_mode(clock, None), None);
    }
}

#[cfg(test)]
mod live_socket_tests {
    use super::*;

    /// Runs the real socket path against the live client API, on the reference machine.
    ///
    /// Ignored by default because it needs a listening client API, and a test with no server
    /// would pass while pinning nothing. This is the only way to check `read_screens` and
    /// `read_client` against a real HTTP response: the parser tests feed them literals.
    ///
    /// Run it with a game in the menus, then in a match, then on the score screen, setting
    /// `SC2SV_EXPECT_MODE` to the mode each state should produce:
    ///
    /// ```text
    /// SC2SV_EXPECT_MODE=analysis cargo test --lib live_socket -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a live client API on port 6119"]
    fn the_socket_path_agrees_with_the_rule_on_a_live_server() {
        let expected = Some(match std::env::var("SC2SV_EXPECT_MODE").as_deref() {
            Ok("overlay") => WindowMode::Overlay,
            Ok("analysis") => WindowMode::Analysis,
            _ => panic!("set SC2SV_EXPECT_MODE to `overlay` or `analysis`"),
        });
        let clock = read_client();
        let screens = if clock.is_some() { read_screens() } else { None };
        eprintln!("clock={clock:?} screens={screens:?}");
        assert!(clock.is_some(), "the client API was not reachable");
        assert_eq!(hotkey_mode(clock, screens), expected);
        assert_eq!(hotkey_mode_now(), expected);
    }
}

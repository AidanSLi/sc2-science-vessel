//! Finding the game's window and placing the overlay relative to it.
//!
//! This is Linux/X11-only on purpose, and it is the reason the app forces GTK onto the X11
//! backend when XWayland is present (see `main.rs`).
//!
//! The problem it solves is that a **native Wayland client cannot do any of the three
//! things the overlay needs**:
//!
//! - `set_always_on_top` is a no-op: GTK3's `gdk_wayland_window_set_keep_above` is an empty
//!   function.
//! - `set_position` is a no-op: there is no protocol for a client to place its own toplevel.
//! - And a Wayland client cannot see another application's window geometry at all, so it
//!   could not follow the game even if it could move itself.
//!
//! StarCraft II under Wine is an **XWayland** window, so X11 is the one protocol where both
//! the game and the overlay are addressable at once. On the X11 backend GTK does honour
//! keep-above, positioning and RGBA visuals (all three verified against KWin on the
//! reference machine), which makes this module possible at all.
//!
//! Nothing here reads the game's memory or sends it input; it only asks the X server for
//! window geometry, which is ordinary window-manager interaction.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt, StackMode, Window};

/// The game's window: its X id and where it is on screen.
///
/// The id is carried alongside the rectangle because keeping the overlay above the game
/// needs the window itself as a stacking sibling, not just its position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameWindow {
    /// The X window id.
    pub id: Window,
    /// The window's rectangle in root coordinates.
    pub rect: Rect,
    /// How confidently the title identifies this as the game; never `0` for a candidate.
    ///
    /// Carried out of [`find_game_window_with`] so a caller inspecting a window can tell an
    /// exact-title match from a prefix one, and so the area tie-break below cannot promote a
    /// larger window over a better-named one.
    pub rank: u8,
}

/// How often the overlay re-checks its position in the stack.
///
/// Fast enough that the overlay is not visibly missing when the player alt-tabs back to the
/// game, slow enough that it costs nothing: one X round trip of a few hundred bytes.
pub const STACKING_POLL: std::time::Duration = std::time::Duration::from_millis(400);

/// A rectangle in root (screen) coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Window-title prefixes that identify the game.
///
/// Matched case-insensitively as a prefix. The window is identified by *title* rather than
/// by `WM_CLASS` because Wine reports the class of the launcher: a StarCraft II started
/// through Steam or Battle.net appears as `steam_app_default`, so class matching would
/// either miss the game or match half of Steam's helper windows.
///
/// A prefix match alone is too loose, which is why [`game_title_rank`] exists: a file manager
/// opened on the replays folder is titled `StarCraft II — Dolphin` and is frequently *larger*
/// than the game, so picking the biggest prefix match anchors the overlay to the file manager.
const GAME_WINDOW_TITLE_PREFIXES: &[&str] = &["starcraft ii", "starcraft 2"];

/// Titles that can only be the game itself, matched case-insensitively.
///
/// Wine gives the window exactly the game's name, so a title that matches one of these exactly
/// outranks any prefix match no matter how large the other window is. This is what keeps a
/// Dolphin window called `StarCraft II — Dolphin` from winning on area.
const GAME_WINDOW_EXACT_TITLES: &[&str] = &["starcraft ii", "starcraft 2"];

/// How well a window title identifies the game — higher is a better candidate.
///
/// Separated from the tree walk so the ranking is testable without an X server, because the
/// ranking is the part that was wrong: an exact title must beat a longer, larger window whose
/// title merely starts with the game's name.
///
/// @param title The window's title.
/// @returns `2` for an exact game title, `1` for a prefix match, `None` for anything else.
fn game_title_rank(title: &str) -> Option<u8> {
    let lower = title.trim().to_ascii_lowercase();
    if GAME_WINDOW_EXACT_TITLES.contains(&lower.as_str()) {
        return Some(2);
    }
    if GAME_WINDOW_TITLE_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return Some(1);
    }
    None
}

/// Smallest window that can plausibly be the game, in pixels per side.
///
/// Wine creates several tiny helper windows whose titles can look game-like (an "Xalia event
/// window", `Default IME` placeholders, a 1x1 hidden toplevel). Requiring a plausible game
/// size filters them out without needing a hard-coded window id.
const MIN_GAME_WINDOW_SIDE: u32 = 400;

/// Title prefix of this application's own window, matched case-insensitively.
///
/// Used only to tell "some client went fullscreen" from "we did", since the app's own
/// fullscreen state is not a reason to stop being an overlay.
const OWN_WINDOW_TITLE_PREFIX: &str = "sc2-science-vessel";

/// Reports whether some client other than this application holds a fullscreen window.
///
/// This exists because a fullscreen client defeats the overlay, and the overlay has to notice
/// rather than sit ordered above a game that is drawing on top of it — the player would see no
/// overlay at all while `always_on_top` claimed otherwise, which is the worst of both. KWin's
/// `Window::belongsToLayer` tests `isActiveFullScreen()` before `keepAbove()`, and
/// `isActiveFullScreen` requires the fullscreen window to be the active one, so an active
/// fullscreen game sits on the `ActiveLayer`, above every layer a `keepAbove` toplevel can
/// reach. The layers that *are* above it (`OverlayLayer`) also cover other applications'
/// fullscreen windows, so going there is a deliberate, user-visible choice (a KWin rule or
/// `wlr-layer-shell`, ADR 0003) rather than something to do silently.
///
/// Implemented by walking the tree rather than by asking the window manager, because there is no
/// portable EWMH query for "is anything fullscreen".
///
/// @returns `true` if another client's viewable window lists `_NET_WM_STATE_FULLSCREEN`.
pub fn another_client_is_fullscreen() -> bool {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return false;
    };
    another_client_is_fullscreen_with(&conn, screen_num)
}

/// [`another_client_is_fullscreen`] over an already-open connection, so it can share one.
///
/// @param conn An open X connection.
/// @param screen_num The screen to search, as returned by `x11rb::connect`.
/// @returns `true` if a client other than this application is fullscreen on that screen.
fn another_client_is_fullscreen_with<C: Connection>(conn: &C, screen_num: usize) -> bool {
    let Some(root) = conn.setup().roots.get(screen_num).map(|screen| screen.root) else {
        return false;
    };
    // `_NET_WM_STATE` holds a list of atoms, so the value to look for must be interned as an
    // atom too rather than compared as a string.
    let Some(fullscreen) = intern_atom(conn, b"_NET_WM_STATE_FULLSCREEN") else {
        return false;
    };
    let Some(net_wm_state) = intern_atom(conn, b"_NET_WM_STATE") else {
        return false;
    };
    let Some(tree) = conn
        .query_tree(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return false;
    };

    let mut stack = tree.children;
    while let Some(window) = stack.pop() {
        if let Ok(cookie) = conn.query_tree(window) {
            if let Ok(children) = cookie.reply() {
                stack.extend(children.children);
            }
        }
        // The app's own fullscreen would be a reason to keep being an overlay, not to stop.
        let own = window_title(conn, window)
            .map(|title| title.to_ascii_lowercase().starts_with(OWN_WINDOW_TITLE_PREFIX))
            .unwrap_or(false);
        if own || !is_viewable(conn, window) {
            continue;
        }
        let fullscreen_found = conn
            .get_property(false, window, net_wm_state, AtomEnum::ATOM, 0, 1024)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| {
                reply
                    .value32()
                    .map(|values| values.into_iter().any(|value| value == fullscreen))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if fullscreen_found {
            return true;
        }
    }

    false
}

/// Interns an atom and returns its id.
///
/// @param conn An open X connection.
/// @param name The atom's name.
/// @returns The atom id, or `None` if the server refused.
fn intern_atom<C: Connection>(conn: &C, name: &[u8]) -> Option<u32> {
    let reply = conn.intern_atom(false, name).ok()?.reply().ok()?;
    Some(reply.atom)
}

/// Returns whether an X server is reachable, i.e. whether XWayland is running.
///
/// Used to decide whether forcing GTK onto the X11 backend will work. A Wayland session
/// without XWayland must not be pushed onto a backend that cannot connect.
///
/// @returns `true` if a connection to the default X display succeeds.
pub fn xwayland_available() -> bool {
    x11rb::connect(None).is_ok()
}

/// Reads a window's `_NET_WM_NAME` (falling back to the legacy `WM_NAME`).
///
/// `_NET_WM_NAME` is UTF-8 and is what modern toolkits set; `WM_NAME` is the ICCCM field and
/// may be Latin-1. Both are tried because a Wine window is not guaranteed to set the former.
///
/// @param conn An open X connection.
/// @param window The window to read.
/// @returns The title, or `None` if the window has none.
fn window_title<C: Connection>(conn: &C, window: Window) -> Option<String> {
    let net_wm_name = conn
        .intern_atom(false, b"_NET_WM_NAME")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let utf8 = conn
        .intern_atom(false, b"UTF8_STRING")
        .ok()?
        .reply()
        .ok()?
        .atom;

    if let Ok(cookie) = conn.get_property(false, window, net_wm_name, utf8, 0, 1024) {
        if let Ok(reply) = cookie.reply() {
            if reply.value_len > 0 {
                if let Ok(text) = String::from_utf8(reply.value) {
                    return Some(text);
                }
            }
        }
    }

    let cookie = conn
        .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
        .ok()?;
    let reply = cookie.reply().ok()?;
    let text = String::from_utf8_lossy(&reply.value).to_string();
    (!text.is_empty()).then_some(text)
}

/// Returns whether a window is actually mapped and viewable.
///
/// Unmapped windows keep their last geometry, so an unfocused-but-cached window would
/// otherwise be a stale anchor that never moves again.
///
/// @param conn An open X connection.
/// @param window The window to test.
/// @returns `true` if the window's map state is `VIEWABLE`.
fn is_viewable<C: Connection>(conn: &C, window: Window) -> bool {
    conn.get_window_attributes(window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|attrs| attrs.map_state == x11rb::protocol::xproto::MapState::VIEWABLE)
        .unwrap_or(false)
}

/// Translates a window's origin into root coordinates.
///
/// The geometry from `get_geometry` is relative to the parent (the window manager's frame),
/// not the screen, so it must be translated. Without this the overlay would drift by the
/// frame's offset — invisible while the window is maximised, wrong the moment it is moved.
///
/// @param conn An open X connection.
/// @param window The window to locate.
/// @returns The window's position in root coordinates, or `None` if unavailable.
fn root_origin<C: Connection>(conn: &C, window: Window) -> Option<(i32, i32)> {
    let tree = conn.query_tree(window).ok()?.reply().ok()?;
    if tree.root == window {
        return Some((0, 0));
    }
    let translated = conn
        .translate_coordinates(window, tree.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((translated.dst_x as i32, translated.dst_y as i32))
}

/// Finds the game's window and returns its rectangle in root coordinates.
///
/// When several windows match the title prefixes, the largest viewable one wins: Wine and
/// Steam both keep extra toplevels whose titles begin with the game's name, and the real one
/// is by far the biggest.
///
/// @returns The game window, or `None` when the game is not running (or the app is not on an
///   X connection).
pub fn find_game_window() -> Option<GameWindow> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    find_game_window_with(&conn, screen_num)
}

/// What one stacking tick learned, so the caller needs only one X connection per tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayPlacement {
    /// The game window's rectangle in root coordinates.
    pub rect: Rect,
    /// Whether some *other* client currently holds a fullscreen window, which defeats
    /// keep-above on KWin and is reported so the caller can stop claiming it.
    pub other_fullscreen: bool,
}

/// Glues `overlay` directly above the game window, returning the game's rectangle.
///
/// This is the mechanism behind "above the game, but not above everything else".
/// `_NET_WM_STATE_ABOVE` is deliberately *not* set while the game is running, because
/// keep-above is a global claim that would also put the overlay over windows the player
/// raised above the game (verified: with the state cleared the overlay drops below every
/// window, with it set it rises above every window). Restacking is the only lever that can
/// express "exactly one window up", so the overlay is re-placed whenever it drifts.
///
/// Anchoring and restacking share one X connection in one call, because the watcher repeats
/// this a few times a second and two connections per tick would be needless.
///
/// @param overlay The overlay's X window id.
/// @returns The game window's rectangle, or `None` when the game is not running.
pub fn anchor_overlay(overlay: Window) -> Option<OverlayPlacement> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let game = find_game_window_with(&conn, screen_num)?;
    // Read the fullscreen state on the same connection: the watcher repeats this a few times a
    // second, and a second connection per tick would be pure overhead.
    let other_fullscreen = another_client_is_fullscreen_with(&conn, screen_num);
    conn.configure_window(
        overlay,
        &ConfigureWindowAux::new()
            .stack_mode(StackMode::ABOVE)
            .sibling(game.id),
    )
    .ok()?;
    // Without the flush the request can sit in the client buffer until the connection is
    // next used, which at a 400 ms poll would make the overlay trail the game visibly.
    conn.flush().ok()?;
    Some(OverlayPlacement {
        rect: game.rect,
        other_fullscreen,
    })
}

/// Returns the X window id behind a Tauri window.
///
/// Tauri implements `raw-window-handle`'s `HasWindowHandle` but does not expose the concrete
/// id, because its type differs per platform; the id is only reachable through that trait.
/// This is called once per show, not per poll, so the connection it costs is irrelevant.
///
/// @param window The Tauri window to identify.
/// @returns The X window id, or `None` when the handle is not an X connection (which is the
///   case on Wayland without XWayland, and on Windows and macOS).
pub fn window_id(window: &tauri::WebviewWindow) -> Option<Window> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => Some(handle.window as Window),
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        _ => None,
    }
}

/// Finds the game window using an already-open X connection.
///
/// Split out from [`find_game_window`] and [`anchor_overlay`] so both can share one
/// connection.
///
/// @param conn An open X connection.
/// @param screen_num The screen to search, as returned by `x11rb::connect`.
/// @returns The game window, or `None` if it is not on screen.
fn find_game_window_with<C: Connection>(conn: &C, screen_num: usize) -> Option<GameWindow> {
    let root = conn.setup().roots.get(screen_num)?.root;
    let tree = conn.query_tree(root).ok()?.reply().ok()?;

    let mut best: Option<GameWindow> = None;
    let mut stack = tree.children;
    while let Some(window) = stack.pop() {
        // Wine nests its windows, so recurse: the titled toplevel is not always a direct
        // child of the root.
        if let Ok(cookie) = conn.query_tree(window) {
            if let Ok(children) = cookie.reply() {
                stack.extend(children.children);
            }
        }

        let Some(title) = window_title(conn, window) else {
            continue;
        };
        let Some(rank) = game_title_rank(&title) else {
            continue;
        };
        if !is_viewable(conn, window) {
            continue;
        }

        let Ok(geometry) = conn
            .get_geometry(window)
            .ok()
            .and_then(|c| c.reply().ok())
            .ok_or(())
        else {
            continue;
        };
        if u32::from(geometry.width) < MIN_GAME_WINDOW_SIDE
            || u32::from(geometry.height) < MIN_GAME_WINDOW_SIDE
        {
            continue;
        }
        let Some((x, y)) = root_origin(conn, window) else {
            continue;
        };

        let candidate = GameWindow {
            id: window,
            rect: Rect {
                x,
                y,
                width: u32::from(geometry.width),
                height: u32::from(geometry.height),
            },
            rank,
        };
        // Rank first, area second: a window whose title is exactly the game's name *is* the
        // game, however small it is next to something that merely starts with the same words.
        // Area only breaks ties within a rank (Wine occasionally keeps duplicate toplevels).
        let better = match best {
            None => true,
            Some(current) => {
                rank > current.rank
                    || (rank == current.rank && area_of(&candidate) > area_of(&current))
            }
        };
        if better {
            best = Some(candidate);
        }
    }

    best
}

/// The area of a candidate, used only to break ties within one title rank.
///
/// @param window A candidate game window.
/// @returns Its area in square pixels.
fn area_of(window: &GameWindow) -> u64 {
    u64::from(window.rect.width) * u64::from(window.rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_whether_an_x_server_is_reachable() {
        // Deliberately not asserting a value: this is the environment probe the caller uses
        // to choose a backend, and its answer legitimately differs between a Wayland session
        // with XWayland and a headless test runner.
        let _ = xwayland_available();
    }

    #[test]
    fn finds_no_game_window_when_no_game_is_running() {
        // The reference machine may or may not have the game open, so this asserts the
        // function is total and does not panic; a found window is equally acceptable.
        if let Some(game) = find_game_window() {
            assert!(game.rect.width >= MIN_GAME_WINDOW_SIDE);
        }
    }

    #[test]
    fn reports_a_fullscreen_state_without_an_x_server() {
        // Totality again: a Wayland-only session must answer `false` rather than panic, since
        // the watcher consults this on every tick.
        let _ = another_client_is_fullscreen();
    }

    /// The exact title is the game, and must outrank a prefix match — this is the Dolphin case
    /// that sent the overlay to the wrong window on the reference machine.
    #[test]
    fn an_exact_title_outranks_a_prefix_match() {
        assert_eq!(game_title_rank("StarCraft II"), Some(2));
        assert!(game_title_rank("StarCraft II") > game_title_rank("StarCraft II — Dolphin"));
    }

    /// Wine's own title casing and surrounding whitespace must not change the verdict.
    #[test]
    fn title_ranking_ignores_case_and_padding() {
        assert_eq!(game_title_rank("  sTaRcRaFt 2 "), Some(2));
        assert_eq!(game_title_rank("STARCRAFT II"), Some(2));
    }

    /// A window that merely mentions the game is a candidate of last resort, not an equal: it
    /// is only ever chosen when no exact-titled window exists.
    #[test]
    fn a_merely_related_title_is_only_a_fallback() {
        assert_eq!(game_title_rank("StarCraft II — Dolphin"), Some(1));
        assert_eq!(game_title_rank("StarCraft II Editor"), Some(1));
    }

    /// Unrelated windows must never be considered at all, or the overlay would anchor to
    /// whatever happened to be on screen.
    #[test]
    fn unrelated_titles_are_not_candidates() {
        assert_eq!(game_title_rank("Spotify Premium"), None);
        assert_eq!(game_title_rank("sc2-science-vessel"), None);
        assert_eq!(game_title_rank(""), None);
    }
}

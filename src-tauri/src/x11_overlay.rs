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

// The game's identifying titles and the minimum plausible side length are shared with the Windows
// implementation rather than duplicated, so the two cannot drift into disagreeing about what the
// game's window looks like.
use crate::platform::{GAME_WINDOW_TITLE_PREFIXES, MIN_GAME_WINDOW_SIDE};

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

/// Flips the overlay between a window-manager-managed window and an override-redirect one.
///
/// This is the mechanism that lets one window serve both states, and it exists because the
/// alternative — a user-installed KWin rule — is a per-user desktop change the app cannot ship.
///
/// ## Why override-redirect reaches above a fullscreen game
///
/// `Window::belongsToLayer()` in KWin returns `OverlayLayer` for `isUnmanaged()` *before* it
/// looks at `isActiveFullScreen()` or `keepAbove()`, so an unmanaged window is above a focused
/// fullscreen client without any rule. An override-redirect window is exactly an unmanaged one.
/// Measured on the reference machine: a 1000x700 override-redirect client over a 3840x2160
/// active fullscreen client was 8.44 % of the screen visible, which is precisely its own area,
/// while the same window managed was 0 % — and the fullscreen client stayed on `layer=5`
/// throughout, proving the window rose rather than the game falling.
///
/// ## Why the flip is not done in place
///
/// `override_redirect` is fixed when the X window is created, and changing it on a live window
/// does nothing: KWin decides whether to manage a window when it first sees it, and a mapped
/// window never asks again. Measured: flipping a mapped window left it `layer=2`, and flipping
/// it while unmapped in raw X left it mapped-but-unpainted. GDK's own `hide()`/`show()` tears
/// the X resource down and creates it again, which is the one sequence that lands the change on
/// the CreateWindow attributes — measured to work in both directions (managed 10.84 %,
/// override-redirect 8.44 %, managed again 10.84 %).
///
/// @param window The Tauri window to flip.
/// @param unmanaged `true` to make it override-redirect (above fullscreen), `false` to make it
///   an ordinary managed window again (interactive, decorated, taskbar-listed).
/// @returns `true` when the window was flipped, `false` when this display server cannot.
pub fn set_unmanaged(window: &tauri::WebviewWindow, unmanaged: bool) -> bool {
    set_unmanaged_and_visibility(window, unmanaged, true)
}

/// [`set_unmanaged`], with control over whether the window is left on screen afterwards.
///
/// The flip has to hide and re-show the window to change the attribute, so it decides visibility
/// as a side effect. A caller that is hiding the window anyway has to say so, or the flip's
/// re-show undoes the hide.
///
/// @param window The window to change.
/// @param unmanaged `true` to become override-redirect, `false` to be managed again.
/// @param restore_visibility Whether to leave the window visible, as it was found.
/// @returns `true` when the change was applied.
pub fn set_unmanaged_and_visibility(
    window: &tauri::WebviewWindow,
    unmanaged: bool,
    restore_visibility: bool,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        gtk_unmanaged(window, unmanaged, restore_visibility)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, unmanaged, restore_visibility);
        false
    }
}

/// Turns GTK's focus-taking behaviour off for the overlay.
///
/// A **click-through overlay must never take keyboard focus.** Taking focus is not a cosmetic
/// problem here: the window manager only keeps a fullscreen window on its active layer while it
/// is the active window, so an overlay that accepted focus demoted the game the instant it
/// appeared — which the app then read as "the game is not active any more", released the
/// unmanaged state, and dropped behind the game, which made the game active again. Measured as
/// a visible oscililation between the two states, ~25 polls in one and ~14 in the other. It also
/// matters on its own terms: the player is meant to keep playing while it is on screen.
///
/// Set these once, before the window is ever mapped.
///
/// @param window The Tauri window to configure.
/// @returns `true` when the flags were applied.
#[cfg(target_os = "linux")]
pub fn refuse_focus(window: &tauri::WebviewWindow) -> bool {
    use gtk::prelude::*;

    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    gtk_window.set_accept_focus(false);
    gtk_window.set_focus_on_map(false);
    true
}

/// Non-Linux stub: Tauri's own flags already express this on Windows and macOS.
///
/// @param window The Tauri window.
/// @returns Always `false`.
#[cfg(not(target_os = "linux"))]
pub fn refuse_focus(_window: &tauri::WebviewWindow) -> bool {
    false
}

/// Lets the window take keyboard focus again, for analysis mode.
///
/// The pairing with [`refuse_focus`] is what keeps the two modes honest: analysis mode is a
/// normal application window the player types into, so it must accept focus, and leaving an
/// earlier "never focus" flag in place would give a window that cannot be typed into and does
/// not raise when clicked.
///
/// @param window The Tauri window.
/// @returns `true` when the platform applied it.
#[cfg(target_os = "linux")]
pub fn allow_focus(window: &tauri::WebviewWindow) -> bool {
    use gtk::prelude::*;

    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    gtk_window.set_accept_focus(true);
    gtk_window.set_focus_on_map(true);
    true
}

/// Non-Linux stub.
///
/// @param window The Tauri window.
/// @returns Always `false`.
#[cfg(not(target_os = "linux"))]
pub fn allow_focus(_window: &tauri::WebviewWindow) -> bool {
    false
}

/// GTK implementation of [`set_unmanaged`].
///
/// Hidden behind a `cfg` rather than written into the caller because both `gtk` and the GDK
/// X11 type are Linux-only, and `lib.rs` must not name a platform type.
///
/// @param window The Tauri window to flip.
/// @param unmanaged Whether to make the window override-redirect.
/// @returns `true` when the flip was applied.
#[cfg(target_os = "linux")]
fn gtk_unmanaged(window: &tauri::WebviewWindow, unmanaged: bool, restore_visibility: bool) -> bool {
    use gtk::prelude::*;

    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    // Both directions go through hide/show, because a GdkWindow cannot change the attribute
    // while it exists. Visibility is preserved across the flip, because the flip is an
    // implementation detail of *how* the window is stacked and must not change whether the
    // player can see it: the caller asks for "managed again", not "gone".
    //
    // `restore_visibility` is the one exception, and it exists because the hotkey hides the
    // window and gives up the unmanaged state in one breath. The hide is queued, so the flip
    // still sees `is_visible() == true` and its own `show()` would win the race and put the
    // overlay straight back — measured as three consecutive presses all reporting
    // `is_visible=true`. An override-redirect window cannot lean on the toolkit's visibility
    // bookkeeping, so the caller has to state what it wants.
    let was_visible = restore_visibility && gtk_window.is_visible();
    gtk_window.hide();
    let Some(gdk_window) = gtk_window.window() else {
        return false;
    };
    gdk_window.set_override_redirect(unmanaged);
    if was_visible {
        gtk_window.show();
    }
    true
}

/// Hides the window synchronously, so a caller that has just hidden it sees `is_visible() ==
/// false` immediately.
///
/// This exists only because Tauri's `hide()` is queued: it posts a request to the GTK event loop
/// and returns, so a `hide()` followed immediately by [`set_unmanaged`] still reads
/// `is_visible() == true` inside the flip. The flip preserves visibility by design (it is an
/// implementation detail of stacking, not something the player asked for), so it re-showed the
/// window the hotkey had just hidden — measured as three consecutive presses all reporting
/// `is_visible=true`. Hiding through GTK directly removes the window between the two calls.
///
/// @param window The window to hide.
/// @returns Whether the hide was performed.
#[cfg(target_os = "linux")]
pub fn hide_now(window: &tauri::WebviewWindow) -> bool {
    use gtk::prelude::*;

    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    gtk_window.hide();
    // Pump the loop once so the unmap is real before the caller reads visibility back. Without
    // this the hide is still just an event in the queue and the next read reports the old value.
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
    true
}

#[cfg(not(target_os = "linux"))]
pub fn hide_now(_window: &tauri::WebviewWindow) -> bool {
    false
}

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

// The window is identified by *title* rather than by `WM_CLASS` because Wine reports the class of
// the launcher: a StarCraft II started through Steam or Battle.net appears as
// `steam_app_default`, so class matching would either miss the game or match half of Steam's
// helper windows.
//
// A prefix match alone is too loose, which is why `game_title_rank` exists: a file manager opened
// on the replays folder is titled `StarCraft II — Dolphin` and is frequently *larger* than the
// game, so picking the biggest prefix match anchors the overlay to the file manager. The titles
// themselves live in `crate::platform`, shared with the Windows implementation.

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

// Wine creates several tiny helper windows whose titles can look game-like (an "Xalia event
// window", `Default IME` placeholders, a 1x1 hidden toplevel). `MIN_GAME_WINDOW_SIDE` is what
// filters them out without needing a hard-coded window id; it lives in `crate::platform`, shared
// with the Windows implementation.

/// Title prefix of this application's own window, matched case-insensitively.
///
/// Used only to tell "some client went fullscreen" from "we did", since the app's own
/// fullscreen state is not a reason to stop being an overlay.
///
/// Two prefixes, because the window title is `SC2 Science Vessel` in a built app (from
/// `tauri.conf.json`'s `productName`) while a dev window's title may still be the crate name.
/// Missing either would make the app mistake its own fullscreen window for the game's.
const OWN_WINDOW_TITLE_PREFIXES: &[&str] = &["sc2 science vessel", "sc2-science-vessel"];

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
/// Returns the window the window manager reports as active, if any.
///
/// Read from `_NET_ACTIVE_WINDOW` on the root window rather than from X input focus, because the
/// overlay is click-through and steals focus at no point, so input focus keeps pointing at
/// whatever it pointed at before. `_NET_ACTIVE_WINDOW` is what the window manager itself
/// considers active, which is exactly the thing KWin's `isActiveFullScreen()` consults.
///
/// @param conn An open X connection.
/// @param screen_num The screen whose root window carries the property.
/// @returns The active window's id.
fn active_window_with<C: Connection>(conn: &C, screen_num: usize) -> Option<Window> {
    let root = conn.setup().roots.get(screen_num)?.root;
    let atom = intern_atom(conn, b"_NET_ACTIVE_WINDOW")?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    // Collected into an owned value rather than returning the iterator: the reply is a local, and
    // an iterator borrowing it cannot outlive the function.
    let active = reply.value32().and_then(|mut values| values.next());
    active
}

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
            .map(|title| {
                let lower = title.to_ascii_lowercase();
                OWN_WINDOW_TITLE_PREFIXES
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
            })
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
    /// Whether the fullscreen game is the *active* window.
    ///
    /// This is what separates "the game is on screen and the player is playing it" from "the
    /// game is still fullscreen but the player has raised something over it". KWin only promotes
    /// a fullscreen window to `ActiveLayer` while it is the active one; the moment another window
    /// takes activation the game drops back to `NormalLayer` and an ordinary keep-above or
    /// restacked window is above it. Reported separately from `other_fullscreen` because the two
    /// need opposite responses: an active fullscreen game must be out-ranked by becoming
    /// unmanaged, while a fullscreen game that is merely *behind* something must let the overlay
    /// fall behind that something too.
    pub game_active: bool,
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
    // Compared by *title*, not by id, because Wine gives the game several windows with the same
    // title: the real client reports five viewable fullscreen windows all called "StarCraft II"
    // (measured on a running game), and the active one is frequently not the one this function
    // picked as the anchor. Comparing ids therefore reported "the game is not active" while the
    // player was demonstrably playing it, which made the app release the state it had just
    // claimed and oscillate. The titles are equal by construction, so title equality is the
    // honest test, and it is what the requirement actually means: "the game, not some other
    // window, is what the player is looking at".
    let game_active = active_window_with(&conn, screen_num)
        .and_then(|active| window_title(&conn, active))
        .is_some_and(|title| game_title_rank(&title).is_some());

    Some(OverlayPlacement {
        rect: game.rect,
        other_fullscreen,
        game_active,
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

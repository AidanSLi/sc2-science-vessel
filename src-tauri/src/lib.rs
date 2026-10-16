//! The Rust shell for sc2-science-vessel.
//!
//! Its job is deliberately thin. It owns the **single** window, the global shortcut, and the
//! lifecycle of the Python analysis sidecar (ADR 0001). It does **not** parse replays, compute
//! metrics, or decide anything about build orders — those live in the sidecar and the frontend.
//!
//! There is one window with two modes, not two windows. `WindowMode::Overlay` makes it a
//! borderless, transparent, click-through guide pinned over the game; `WindowMode::Analysis`
//! restores an ordinary resizable window. `apply_window_mode` is the only place allowed to set
//! those properties, because they do not reliably survive a hide/show cycle. The mode is chosen
//! from the client's `/ui` screen list (ADR 0010), and visibility is the hotkey's business
//! alone — the two are deliberately independent, so an automatic mode change never resurrects a
//! window the player hid.
//!
//! Two constraints from the feasibility study shape this file and must not be broken:
//!
//! - The app never reads the game process's memory, and never sends input to the game.
//!   The only sanctioned live input is the SC2 Client API on `localhost:6119`, which the
//!   frontend polls directly (ADR 0004). This is what keeps the overlay on the benign
//!   side of Blizzard's discretionary EULA clause.
//! - The window is a plain always-on-top, click-through toplevel for now, positioned by
//!   restacking over the game's X11 window (ADR 0008). KDE exposes `zwlr_layer_shell_v1`, so a
//!   real compositor-level overlay surface is reachable and is the planned upgrade (ADR 0003).

use std::path::Path;
use tauri::{Emitter, Manager, WebviewWindow};

#[cfg(target_os = "linux")]
mod portal_shortcut;
mod game_watch;
mod notify;
mod settings;
mod sidecar;
pub mod x11_overlay;

pub use settings::Settings;

/// The app's single window, whose role changes between analysis mode and overlay mode.
///
/// There is deliberately **one** window. An earlier design used two (a main window plus a
/// separate always-on-top overlay) and it was wrong for a reason worth recording: two windows
/// means two things that can be on screen at once, two stacking relationships to negotiate,
/// and a toggle that has to decide which one it applies to. The player experiences one window
/// that changes shape, so that is what the code models.
///
/// The mode is *owned here*, not by the frontend, because it decides window properties the
/// frontend cannot set (decorations, click-through, size) and because it must be consistent
/// with the global hotkey, which arrives from the compositor through Rust without the
/// frontend being involved.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// Which of its two roles the single window is playing.
///
/// Serialised to the frontend as the lowercase tag (`"analysis"` / `"overlay"`), which is the
/// same spelling the `set_window_mode` command accepts, so a mode can be round-tripped through
/// a command argument without a second vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    /// Ordinary resizable, decorated, interactive window: matches, builds, analysis, settings.
    Analysis,
    /// Borderless, transparent, click-through build-order guide pinned over the game.
    Overlay,
}

/// The window's current mode.
///
/// Process-wide state rather than per-call, because the global hotkey, the game-state watcher
/// and the frontend's own toggle command all need to agree on it. A plain `Mutex<Option<...>>`
/// would do; the `AtomicU8` avoids holding a lock across the window calls, which can block on
/// the compositor.
static WINDOW_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Reads the window's current mode.
///
/// @returns The mode the window is in.
fn window_mode() -> WindowMode {
    match WINDOW_MODE.load(std::sync::atomic::Ordering::SeqCst) {
        1 => WindowMode::Overlay,
        _ => WindowMode::Analysis,
    }
}

/// Records the window's mode.
///
/// @param mode The mode the window has just been put into.
fn set_window_mode(mode: WindowMode) {
    let raw = match mode {
        WindowMode::Analysis => 0,
        WindowMode::Overlay => 1,
    };
    WINDOW_MODE.store(raw, std::sync::atomic::Ordering::SeqCst);
}

/// Switches the window between analysis mode and overlay mode.
///
/// This is the single choke point for window geometry and window flags. Every caller — the
/// global hotkey, the frontend's button, the game-state watcher — goes through here, so the
/// window can never end up borderless-but-clickable or, worse, transparent-but-interactive
/// over the game.
///
/// The flags are re-applied on every mode change rather than only at creation because
/// `always_on_top` and cursor passthrough are not guaranteed to survive a re-show, and a
/// click-through overlay that has quietly become clickable is a bug the player would only
/// notice mid-game.
///
/// @param app The running app handle.
/// @param mode The mode to switch to.
/// @returns The mode now in effect.
#[tauri::command]
fn set_window_mode_command(app: tauri::AppHandle, mode: String) -> tauri::Result<WindowMode> {
    let target = match mode.as_str() {
        "overlay" => WindowMode::Overlay,
        _ => WindowMode::Analysis,
    };
    // `AsIs`, not `Shown`: this command reports the mode the frontend should render, and
    // deciding to reveal the window is the hotkey's job. A player who hides the app and then
    // has the mode change underneath them must not have the window pop back up.
    apply_window_mode(&app, target, Visibility::AsIs)?;
    Ok(target)
}

/// Reads the window's current mode, so the frontend can render the matching view.
///
/// The frontend cannot infer this: the hotkey changes the mode without any frontend
/// involvement, so the mode has to be asked for rather than derived from a local variable.
///
/// @returns The mode the window is in.
#[tauri::command]
fn get_window_mode() -> WindowMode {
    window_mode()
}

/// Shows or hides the app window, leaving its mode alone.
///
/// **This is what the global hotkey does.** The hotkey is a visibility control, not a mode
/// switch: the player presses it to bring the app up quickly after a game, and the app shows
/// whichever view the game state calls for at that moment. Making the hotkey choose a mode
/// instead would mean a player mid-game could press it and land on the analysis pages, which
/// is both useless during a game and a way to lose the build order they were following.
///
/// @param app The running app handle.
/// @returns `true` if the window is now visible.
#[tauri::command]
fn toggle_window(app: tauri::AppHandle) -> tauri::Result<bool> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(false);
    };

    if window.is_visible().unwrap_or(false) {
        window.hide()?;
        // Stop the stacking watcher while hidden: it exists to keep the overlay above the
        // game, and a hidden window has nothing to stack. Leaving it running would also keep
        // an X connection busy and re-assert keep-above on a window nobody can see.
        OVERLAY_WATCH_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(false)
    } else {
        show_window(&app)?;
        Ok(true)
    }
}

/// Shows the app window, re-applying whatever its current mode needs.
///
/// A show is not just `show()`: the flags that make overlay mode usable (click-through, the
/// stacking watcher) are lost when a window is hidden, and a click-through overlay that has
/// quietly become clickable is a bug the player would only notice mid-game. Re-applying the
/// whole mode is the cheapest way to guarantee that cannot happen.
///
/// @param app The running app handle.
fn show_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    apply_window_mode(app, window_mode(), Visibility::Shown)
}

/// Makes a hidden start behave, and keeps a non-hidden start honest.
///
/// ## The show is not optional, and is not a bug
///
/// The app is resident: it is meant to sit hidden until the player finishes a game and presses
/// the hotkey. A WebView that has **never been shown** is not equivalent to one that has been
/// shown and then hidden: Tauri's issue #15652 records a window created with `visible: false`
/// permanently failing to receive events — `listen()` resolves, the JS never fires, and an
/// undeliverable `EvaluateScript` is discarded with no log. wry works around the same problem
/// internally in its GTK backend, calling `show_all()` and then hiding again because "if the
/// webview starts as hidden, we will need about 3 calls to `webview.set_visible`".
///
/// So the window is shown and immediately hidden on every start. On a hidden start that is the
/// whole point — it is what makes the first real reveal instant instead of a blank frame — and
/// on a visible start it is a no-op the reveal immediately undoes. Doing it in both cases keeps
/// one code path, and the cost is a window that may flash for a frame on a visible start.
///
/// Order matters: the reveal happens last, so a visible start never ends up hidden by a racing
/// hide.
///
/// @param app The running app handle.
fn warm_or_reveal_main_window(app: tauri::AppHandle) {
    let start_hidden = Settings::load(&app).start_hidden;
    std::thread::spawn(move || {
        let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        // Map it once so the webview is painted and its event delivery is known to work.
        let _ = window.show();
        let _ = window.hide();

        if !start_hidden {
            let _ = window.show();
        }
    });
}

/// Whether putting the window into a mode should also make it visible.
///
/// This exists because **mode and visibility are independent**, and conflating them is a bug
/// with a specific symptom: the player hides the app with the hotkey during a game, the game
/// ends, the automatic mode change fires, and the app jumps back on screen uninvited. The
/// player asked for it to be gone; only the hotkey may bring it back.
///
/// So the two callers differ, deliberately. A hotkey-driven show wants [`Visibility::Shown`],
/// because the player is asking to see the app and must see it in the right shape. An
/// automatic mode change wants [`Visibility::AsIs`]: it changes what the window *is*, and
/// leaves the question of whether it is on screen exactly where the player left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    /// Leave the window's visibility exactly as it is.
    AsIs,
    /// Make the window visible.
    Shown,
}

/// Decides whether a mode change should also make the window visible.
///
/// A plain function so the rule can be tested without a window, because it is the rule that
/// was wrong: an earlier version revealed the window on every mode change, so hiding the app
/// during a game and then finishing the game would pop the window back up over a player who
/// had asked for it to be gone.
///
/// @param visibility What the caller asked for.
/// @param currently_visible Whether the window is visible right now.
/// @returns `true` when the window should be shown as part of the change.
fn should_reveal(visibility: Visibility, currently_visible: bool) -> bool {
    match visibility {
        Visibility::Shown => true,
        Visibility::AsIs => currently_visible,
    }
}

/// Puts the single window into the given mode, optionally revealing it.
///
/// Ordering matters and is not arbitrary: in overlay mode the window is made click-through
/// **after** being shown, because `set_ignore_cursor_events` unwraps the underlying `GdkWindow`
/// on Linux and that does not exist until the toplevel has been realised (ADR 0007). That is
/// why [`Visibility::AsIs`] still shows an already-visible window — the call is what keeps a
/// shown overlay's flags correct — and why the click-through call can never move above it.
///
/// In analysis mode the reverse order is what matters: click-through is cleared first, so the
/// window is never interactive-but-transparent for even a frame.
///
/// The size constraints are set **per mode**, from the one place that knows both sizes. This
/// is not fussiness: a minimum is a hard floor the window manager enforces, so leaving the
/// analysis floor in place while sizing the window down to the 380x300 overlay would have it
/// clamped straight back to 900 wide, and the overlay would silently be the wrong size. The
/// constraints therefore have to move with the size, in the same function, or they fight it.
///
/// @param app The running app handle.
/// @param mode The mode to apply.
/// @param visibility Whether to reveal the window as part of the change.
fn apply_window_mode(
    app: &tauri::AppHandle,
    mode: WindowMode,
    visibility: Visibility,
) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(());
    };

    // Read once, at the top, before any branch touches the window. Every branch below needs to
    // know whether to reveal it, and re-reading after a `hide` would answer the wrong question.
    let should_show = should_reveal(visibility, window.is_visible().unwrap_or(false));

    match mode {
        WindowMode::Overlay => {
            // Pinned to exactly the overlay size, not merely freed from the analysis floor.
            // Clearing the constraints is not enough: `tauri.conf.json` sets `minWidth`/
            // `minHeight` on the window, so the floor is re-asserted by the toolkit and the
            // overlay ends up clamped to whatever minimum is still in force (measured: the
            // overlay came up 1024x656 instead of 380x300 at a 1.25 scale factor). Pinning both
            // bounds makes the size a fact rather than a request that something else can refuse.
            window.set_min_size(Some(tauri::LogicalSize::new(OVERLAY_WIDTH, OVERLAY_HEIGHT)))?;
            window.set_max_size(Some(tauri::LogicalSize::new(OVERLAY_WIDTH, OVERLAY_HEIGHT)))?;
            // Clearing click-through first matters when switching *out of* overlay mode and
            // back: the window would otherwise be shown before the flag is cleared and be
            // transparent-but-unclickable for a frame.
            window.set_ignore_cursor_events(false)?;
            window.set_decorations(false)?;
            window.set_shadow(false)?;
            window.set_skip_taskbar(true)?;
            if should_show {
                window.show()?;
            }
            // Every request below needs a realised `GdkWindow`, so they all come after
            // `show` — and that includes the size. A pre-show `set_size` is a race, not a
            // guarantee: GTK queues it against a window that does not exist yet and the
            // request is lost whenever the window is not being mapped for the first time.
            // Measured on start-up into overlay mode: the pre-show call left the window at
            // its configured 1280x820 (`inner_size` read back 1280x820 after `show`), while
            // the same call repeated after the show took effect immediately (380x300). This
            // is the same class of bug as click-through and always-on-top, so the fix is the
            // same: order the request after the mapping, not before it.
            window.set_size(tauri::LogicalSize::new(OVERLAY_WIDTH, OVERLAY_HEIGHT))?;
            // Deliberately NOT `set_resizable(false)`: GTK reads a non-resizable window as
            // "size yourself to your content", and the WebView's natural size then beats both
            // an explicit `set_size` and pinned min/max geometry hints. Measured on a cold
            // start into overlay mode: with the flag set the window settled at 1280x2108 (the
            // WebView's natural size) and KWin reported 1024x1686; leaving it alone produced
            // the intended 380x300 / 304x240 every time. Nothing is lost, because overlay mode
            // is also click-through — every pointer event passes through the window, so its
            // edges cannot be dragged in the first place.
            reposition_for_overlay(&window, None);
            window.set_always_on_top(true)?;
            window.set_ignore_cursor_events(true)?;
            // Only watch stacking for a visible overlay: the watcher's whole job is to express
            // "above the game", and a hidden window has nothing to stack.
            #[cfg(target_os = "linux")]
            if should_show {
                watch_overlay_stacking(window.clone());
            }
        }
        WindowMode::Analysis => {
            // Stop the stacking watcher before anything else: it re-asserts the overlay's
            // stacking and keep-above state several times a second, and would fight the mode
            // change if it kept running.
            OVERLAY_WATCH_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
            // The interactive floor, applied before the resize so the window cannot be briefly
            // smaller than the pages it is about to show. The upper bound is a safety net
            // rather than a design choice: a window taller than the work area cannot be moved
            // back into view on every desktop, so a stray resize cannot leave the app stuck.
            window.set_min_size(Some(tauri::LogicalSize::new(
                ANALYSIS_MIN_WIDTH,
                ANALYSIS_MIN_HEIGHT,
            )))?;
            window.set_max_size(Some(tauri::LogicalSize::new(
                ANALYSIS_MAX_WIDTH,
                ANALYSIS_MAX_HEIGHT,
            )))?;
            window.set_ignore_cursor_events(false)?;
            window.set_always_on_top(false)?;
            window.set_decorations(true)?;
            window.set_shadow(true)?;
            window.set_resizable(true)?;
            window.set_skip_taskbar(false)?;
            if should_show {
                window.show()?;
            }
            // Every geometry request below needs a realised `GdkWindow`, and a size request
            // against an unrealised one is a race rather than a guarantee — see the overlay
            // branch for the measurement. An automatic mode change leaves the window hidden;
            // for a hidden window the constraints alone are what will apply when it is next
            // shown, so skipping the resize and the re-centre there loses nothing.
            if should_show {
                window.set_size(tauri::LogicalSize::new(ANALYSIS_WIDTH, ANALYSIS_HEIGHT))?;
                // Only re-centre a window that the player is asking to see. Re-centring one
                // they had deliberately placed would move it on every automatic scan of
                // client state.
                if visibility == Visibility::Shown {
                    window.center()?;
                }
            }
            // Only steal focus when the player asked for the window. An automatic mode change
            // must never pull focus away from a running game.
            if visibility == Visibility::Shown {
                window.set_focus()?;
            }
        }
    }

    set_window_mode(mode);
    // The frontend has to be told, because the mode can change without it: the global hotkey
    // arrives from the compositor straight into this module. Emitting after the mode is stored
    // means any listener reading `get_window_mode` immediately still sees the new value.
    let _ = app.emit(WINDOW_MODE_EVENT, mode);
    Ok(())
}

/// Event name the shell emits when the window's mode changes.
///
/// A named constant shared with the frontend by convention: the frontend's listen call and
/// this emit must agree exactly, and a typo would present as a build guide that never appears
/// after a hotkey press.
pub const WINDOW_MODE_EVENT: &str = "window-mode";

/// Places the window for overlay mode: over the game's top-left corner, or a fixed fallback.
///
/// A fallback is needed because the player may toggle the overlay before the game has
/// started, which is the ordinary case rather than an error.
///
/// @param window The window to move.
/// @param game The game's rectangle when it is known. The stacking watcher passes the one it
///   has already read, so a move does not re-walk the window tree and open a second X
///   connection 2.5 times a second for a value it holds.
fn reposition_for_overlay(window: &WebviewWindow, game: Option<x11_overlay::Rect>) {
    #[cfg(target_os = "linux")]
    {
        let (x, y) = overlay_position(window, game);
        if let Err(err) = window.set_position(tauri::LogicalPosition::new(x, y)) {
            eprintln!("could not move the overlay ({err})");
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, game);
    }
}

/// Whether the overlay should be claiming the keep-above state this tick.
///
/// A pure function so the decision is testable: the two behaviours that matter — yielding to a
/// fullscreen client, and dropping the claim when the game is gone — are otherwise observable
/// only by watching KWin. Restating the rule: keep-above holds only while the game is found and
/// nobody else is fullscreen.
///
/// @param other_fullscreen Whether another client holds an active fullscreen window.
/// @param game_anchored Whether the game window was found and restacked against.
/// @returns Whether the window should be always-on-top.
#[cfg(target_os = "linux")]
fn desired_keep_above(other_fullscreen: bool, game_anchored: bool) -> bool {
    !other_fullscreen && game_anchored
}

/// Applies [`desired_keep_above`] to the window, touching it only on a transition.
///
/// The flag is a window property, so setting it on every poll would send a request per tick for
/// a value that is almost always unchanged. The position is different: the player may drag or
/// resize the game at any time, and the overlay is meant to sit in the game's top-left corner, so
/// the move is attempted every poll and skipped at the X level when nothing has changed.
///
/// @param window The overlay window.
/// @param other_fullscreen Whether another client holds an active fullscreen window.
/// @param game_anchored Whether the game window was found and restacked against.
/// @param keep_above The watcher's current belief about the window's always-on-top state.
/// @returns The state the caller should carry into the next poll.
#[cfg(target_os = "linux")]
fn apply_overlay_stacking(
    window: &WebviewWindow,
    other_fullscreen: bool,
    game_anchored: bool,
    keep_above: bool,
) -> bool {
    let next = desired_keep_above(other_fullscreen, game_anchored);
    if next != keep_above {
        if next {
            // Re-showing is part of retaking the top: a window left behind another client's
            // fullscreen may have been obscured long enough that the compositor will not
            // re-evaluate it.
            let _ = window.show();
        }
        let _ = window.set_always_on_top(next);
    }
    next
}
/// Keeps the overlay anchored to the game and directly above it, until the thread is told to
/// stop.
///
/// A thread rather than a Tauri interval because the work is a plain X round trip with no
/// interaction with the app's state, and because the thing it replaces — `_NET_WM_STATE_ABOVE`
/// — cannot be expressed as a window property at all (see `x11_overlay::anchor_overlay`).
///
/// **This is what makes "always above the game, still below anything the player raises over it"
/// true.** A window in the keep-above state floats over every other window, including ones the
/// player deliberately brought forward; restacking expresses the narrower relationship that was
/// actually asked for.
///
/// The watcher also yields to a fullscreen client, because that relationship is not achievable
/// here at all and pretending otherwise breaks the app. KWin's `Window::belongsToLayer` checks
/// `isActiveFullScreen()` **before** `keepAbove()`, and `isActiveFullScreen` requires the
/// fullscreen window to be the active one, so a genuinely focused fullscreen game sits on the
/// `ActiveLayer` — above every layer a `keepAbove` toplevel can reach. No stacking request
/// survives that, and the only way up is into `OverlayLayer` (a KWin rule or wlr-layer-shell,
/// ADR 0003), which would also cover other applications' fullscreen windows. So while another
/// client is fullscreen the watcher drops keep-above and re-shows the window: a window that is
/// behind the game but can still be raised and read is far more useful than one that is
/// invisible and yet still swallowing clicks. Keep-above returns as soon as it goes away.
///
/// @param window The overlay window.
#[cfg(target_os = "linux")]
fn watch_overlay_stacking(window: WebviewWindow) {
    use std::sync::atomic::Ordering;

    // A process-wide flag rather than a per-call one: only one watcher may exist, and the
    // atomic swap in `start_overlay_watcher` is what enforces that. Two watchers would fight
    // over the same stacking slot several times a second.
    if OVERLAY_WATCH_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || {
        let Some(id) = x11_overlay::window_id(&window) else {
            // A Wayland-only session has no X ids, so there is nothing to stack against; the
            // caller has already warned that the overlay cannot stay above the game there.
            OVERLAY_WATCH_RUNNING.store(false, Ordering::SeqCst);
            return;
        };

        // Only changed on a transition: the flag is a window property, and setting it every
        // 400 ms would send a request per tick for a value that is almost always unchanged.
        let mut keep_above = true;
        // The last game rect the overlay was placed against, so a poll that finds the game in the
        // same place does not issue a move request. Comparing here rather than inside
        // `set_position` keeps the decision where the previous state lives.
        let mut last_anchor: Option<(i32, i32)> = None;
        while OVERLAY_WATCH_RUNNING.load(Ordering::Relaxed) {
            if window.is_visible().unwrap_or(false) {
                match x11_overlay::anchor_overlay(id) {
                    Some(placement) => {
                        keep_above = apply_overlay_stacking(
                            &window,
                            placement.other_fullscreen,
                            true,
                            keep_above,
                        );
                        // Moving the game must move the overlay with it, otherwise the guide
                        // drifts away from the corner the player expects to find it in the
                        // moment they drag the game window.
                        let anchor = (placement.rect.x, placement.rect.y);
                        if last_anchor != Some(anchor) {
                            reposition_for_overlay(&window, Some(placement.rect));
                            last_anchor = Some(anchor);
                        }
                    }
                    // No game window: stop claiming the top so the overlay does not float over
                    // unrelated windows while the player is doing something else.
                    None => {
                        keep_above = apply_overlay_stacking(&window, false, false, keep_above);
                        last_anchor = None;
                    }
                }
            }
            std::thread::sleep(x11_overlay::STACKING_POLL);
        }
    });
}

/// Where to put the overlay, preferring the game window's top-left corner.
///
/// Anchoring to the game rather than to the screen matters because the game is not always
/// where a fixed coordinate assumes: StarCraft II opens at whatever position and size the
/// player last used, and under Wine that is frequently neither maximised nor at the origin.
/// An overlay pinned to a fixed screen corner would sit off the game entirely — or, on a
/// multi-monitor desktop, on the wrong monitor.
///
/// Falls back to a fixed position when the game is not found, so the overlay still opens
/// somewhere predictable rather than not at all.
///
/// @param window The overlay window, used to read its own scale factor.
/// @param game The game's rectangle when already known, or `None` to look it up here.
/// @returns The position to move the overlay to, in logical pixels.
#[cfg(target_os = "linux")]
fn overlay_position(window: &WebviewWindow, game: Option<x11_overlay::Rect>) -> (f64, f64) {
    if let Some(game) = game.or_else(|| x11_overlay::find_game_window().map(|found| found.rect)) {
        // X11 coordinates are physical pixels while Tauri positions in logical pixels, so on
        // a scaled display (the reference machine runs 3840x2160 at 200%) an unconverted
        // offset would place the overlay at twice the intended distance.
        let scale = window.scale_factor().unwrap_or(1.0);
        if scale > 0.0 {
            return (
                (game.x as f64 + f64::from(OVERLAY_INSET)) / scale,
                (game.y as f64 + f64::from(OVERLAY_INSET)) / scale,
            );
        }
    }

    (f64::from(OVERLAY_INSET), f64::from(OVERLAY_INSET) + 72.0)
}

/// How long to wait for the frontend's ready signal before showing the window anyway.
///
/// Generous on purpose: a first run on a cold cache spends several seconds in Vite's
/// `Forced re-optimization of dependencies`, and a window that never appears is worse than
/// one that appears a moment late.
const MAIN_WINDOW_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(10);

/// Shows the main window. Idempotent, and safe to call from any thread.
///
/// @param app The running app handle.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        // `is_visible` can error transiently on a window being torn down; showing it again
        // is harmless, so a failure here is not worth propagating.
        if !window.is_visible().unwrap_or(false) {
            let _ = window.show();
        }
    }
}

/// Shows the main window if the frontend has not done so within [`MAIN_WINDOW_WATCHDOG`].
///
/// The main window is declared `visible: false` so it cannot be seen before the frontend has
/// painted. Without this timer the app would have a single point of failure: if the frontend
/// never loads (a wrong `devUrl`, a failed asset load, a JavaScript error before first paint)
/// nothing would ever show the window, and the app would look like it had not started at all.
/// The timer bounds that to "a slow start" instead of "nothing happens".
///
/// **It does not run on a resident start.** A resident app is supposed to be invisible, so a
/// watchdog that forced the window on screen ten seconds after launch would undo the feature it
/// is meant to protect. The trade is deliberate: on such a start a frontend that never loads
/// leaves no visible window, which is correct for the mode the user asked for, and the app can
/// still be opened with the hotkey or by launching it again.
///
/// @param app The running app handle.
fn arm_main_window_watchdog(app: tauri::AppHandle) {
    if Settings::load(&app).start_hidden {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(MAIN_WINDOW_WATCHDOG);
        show_main_window(&app);
    });
}

/// Command the frontend calls once it has painted, to reveal the main window.
///
/// Deliberately not an event listener: the frontend's first paint is the signal, so the
/// command is invoked from the root layout's `onMount`, and a command is the smallest
/// thing that can cross the IPC boundary from a page that has no Rust-side state to read.
/// It is also idempotent — the watchdog may have already shown the window.
///
/// **A resident start makes this a no-op.** The whole point of `start_hidden` is that the app
/// sits invisibly until the player asks for it, so a frontend that has painted must not reveal
/// the window on its own. The player's hotkey is the only thing that may bring it up.
#[tauri::command]
fn frontend_ready(app: tauri::AppHandle) {
    if Settings::load(&app).start_hidden {
        return;
    }
    show_main_window(&app);
}

/// Returns the user's settings for the settings page to render.
///
/// The frontend cannot read the config directory itself, so settings live in Rust and are
/// fetched over IPC. `overlay_shortcut` is the *registered* key, which may differ from what
/// the user last asked for if that key was taken (see `register_overlay_shortcut`) — the UI
/// must show the truth, or the player would keep pressing a dead combination mid-game.
///
/// @param app The running app handle.
/// @returns The current settings.
#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Settings {
    Settings::load(&app)
}

/// Rebinds the overlay toggle key and persists it.
///
/// Re-registers immediately rather than deferring to the next launch, because the whole point
/// of changing the key is to use it after the game has already started. Falls back through
/// the same candidate list as startup when the requested key is taken, and returns the key
/// that ended up registered so the UI can report the substitution instead of silently
/// disagreeing with what the player pressed.
///
/// @param app The running app handle.
/// @param shortcut The requested accelerator, e.g. `Ctrl+Alt+O`.
/// @returns The registered accelerator, or `Err` with every candidate's failure reason.
#[tauri::command]
fn set_overlay_shortcut(app: tauri::AppHandle, shortcut: String) -> Result<String, String> {
    let registered = settings::register_overlay_shortcut(&app, &shortcut)?;
    let mut current = Settings::load(&app);
    current.overlay_shortcut = registered.clone();
    current.save(&app)?;
    Ok(registered)
}

/// Returns directories that look like replay libraries, so the first run can be one click.
///
/// @returns Existing candidate paths, possibly empty.
#[tauri::command]
fn suggest_replay_roots() -> Vec<String> {
    settings::suggest_replay_roots()
}

/// Replaces the configured replay roots.
///
/// Takes the whole list rather than adding or removing one entry at a time. The list is small
/// and the user edits it as a set, so a whole-list write cannot produce a half-applied state
/// if the app dies mid-edit, and it keeps the persisted order the same as the order shown.
///
/// Paths are stored as absolute paths where they resolve, because a relative path in the
/// settings file would mean something different depending on how the app was launched.
/// Non-existent paths are still stored: a user may register a network or removable drive that
/// is not mounted right now, and silently dropping it would lose their configuration.
///
/// @param app The running app handle.
/// @param roots The complete list of directories.
/// @returns The stored list.
#[tauri::command]
fn set_replay_roots(app: tauri::AppHandle, roots: Vec<String>) -> Result<Vec<String>, String> {
    let normalised: Vec<String> = roots
        .into_iter()
        .map(|root| root.trim().to_string())
        .filter(|root| !root.is_empty())
        .map(|root| {
            // `canonicalize` also resolves symlinks, which would rewrite a user's carefully
            // chosen path into something they did not type, so only a relative path is made
            // absolute. `std::path::absolute` would do this in one call but it needs Rust 1.79
            // and this crate's MSRV is 1.77.2, so the join is spelled out.
            let path = Path::new(&root);
            if path.is_absolute() {
                root
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path).display().to_string())
                    .unwrap_or(root)
            }
        })
        .collect();

    let mut current = Settings::load(&app);
    current.replay_roots = normalised.clone();
    current.save(&app)?;
    Ok(normalised)
}

/// Sets the build order loaded for each matchup.
///
/// Takes the whole map rather than one matchup at a time, for the same reason
/// `set_replay_roots` takes the whole list: it is small, it is edited as a set, and a crash
/// mid-edit cannot leave a half-applied preset. Keys and ids are trimmed of whitespace and
/// empty entries are dropped, so clearing a dropdown in the UI actually clears the preset
/// instead of storing an empty id that matches nothing.
///
/// Neither keys nor ids are validated against the build library here. The library lives in
/// the frontend bundle, and a preset naming a build that no longer exists after a snapshot
/// refresh is a normal outcome that the overlay already handles by falling back.
///
/// # Arguments
///
/// * `presets` - Matchup (or `ZvX` wildcard) to build order id.
///
/// # Returns
///
/// The stored map, with empty entries removed.
#[tauri::command]
fn set_build_presets(
    app: tauri::AppHandle,
    presets: std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let cleaned = settings::clean_presets(presets);
    let mut current = Settings::load(&app);
    current.build_presets = cleaned.clone();
    current.save(&app)?;
    Ok(cleaned)
}

/// Turns the resident start on or off and persists it.
///
/// Turning it **on takes effect immediately**: the window is hidden here, because a setting
/// that claims the app is resident while the window sits on screen would be lying. Turning it
/// off only changes what happens at the next launch — the player is looking at the window, and
/// hiding it in response to a switch they turned *off* would be perverse.
///
/// @param app The running app handle.
/// @param enabled Whether the app should start hidden.
/// @returns The stored value.
#[tauri::command]
fn set_start_hidden(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let mut current = Settings::load(&app);
    current.start_hidden = enabled;
    current.save(&app)?;
    if enabled {
        if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
            let _ = window.hide();
            OVERLAY_WATCH_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    Ok(enabled)
}

/// Turns "launch at login" on or off and persists it, applying it to the desktop now.
///
/// Refused for a build where it cannot work — see [`Settings::autostart_available`], whose
/// reasoning is the reason this returns an error instead of silently writing a login entry
/// pointing at a `target/debug` binary that the next `cargo` run will replace.
///
/// The autostart plugin is the only thing that writes the desktop's autostart entry, so the
/// stored setting and the desktop are changed together; if the desktop change fails, the
/// setting is not persisted, or the settings page and the user's login behaviour would
/// disagree.
///
/// @param app The running app handle.
/// @param enabled Whether to launch the app at login.
/// @returns The stored value.
#[tauri::command]
fn set_launch_at_login(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;

    if !Settings::autostart_available(&app) {
        return Err(
            "launch at login needs an installed build; run the app from a bundle or package \
             rather than from a development binary"
                .to_string(),
        );
    }

    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|err| err.to_string())?;
    } else {
        autolaunch.disable().map_err(|err| err.to_string())?;
    }

    let mut current = Settings::load(&app);
    current.launch_at_login = enabled;
    current.save(&app)?;
    Ok(enabled)
}

/// Builds the tray icon, which is how a resident app is reached while its window is hidden.
///
/// Four entries, because they are the four things a player does with an app they cannot see:
/// open the build-order guide, open the analysis window, reach the settings, and quit. Without
/// a tray icon the app would be running with no visible affordance at all, and the only way to
/// stop it would be to find it in a task manager — which is not acceptable for something that
/// starts with the session.
///
/// The menu events deliberately reuse the same entry points as the rest of the app rather than
/// duplicating window logic here, so the tray cannot drift out of step with what the commands
/// do.
///
/// @param app The running app handle.
/// @returns `Err` with a human-readable reason if the icon could not be created.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let overlay = MenuItem::with_id(app, "overlay", "Show overlay", true, None::<&str>)?;
    let analysis = MenuItem::with_id(app, "analysis", "Show analysis", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&overlay, &analysis, &settings, &quit])?;

    TrayIconBuilder::new()
        // The 32x32 icon is already in the bundle's icon set, so no new asset is needed.
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("the bundle declares no default window icon".into())
        })?)
        .menu(&menu)
        // Left-click opening the menu is the convention on Linux; the appindicator backend
        // does not deliver left-click events to the app at all.
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "overlay" => {
                if let Err(err) = apply_window_mode(app, WindowMode::Overlay, Visibility::Shown) {
                    eprintln!("could not show the overlay from the tray ({err})");
                }
            }
            "analysis" => {
                if let Err(err) = apply_window_mode(app, WindowMode::Analysis, Visibility::Shown) {
                    eprintln!("could not show the analysis window from the tray ({err})");
                }
            }
            "settings" => {
                // Shows the window first: the settings page lives inside it, and the frontend
                // cannot navigate a window nobody can see.
                if let Err(err) = apply_window_mode(app, WindowMode::Analysis, Visibility::Shown) {
                    eprintln!("could not show the window for settings ({err})");
                } else {
                    let _ = app.emit("navigate", "settings");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Builds and runs the application.
///
/// Registers the plugins the shell needs and wires the global shortcut that toggles the
/// overlay. The shortcut is loaded from the user's settings file, with the default and the
/// fallback list living in `settings.rs` — it is a user-visible setting (the settings page
/// exposes it) rather than a constant, because the right key depends on what else the
/// player's desktop already binds.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // No extra arguments: the app knows it is resident from its own settings, so the
            // login launch must not need a flag to behave correctly.
            None,
        ))
        // Reveals the main window once the frontend has painted (see `reveal_main_window`).
        .invoke_handler(tauri::generate_handler![
            frontend_ready,
            toggle_window,
            get_window_mode,
            set_window_mode_command,
            get_settings,
            set_overlay_shortcut,
            set_start_hidden,
            set_launch_at_login,
            suggest_replay_roots,
            set_replay_roots,
            set_build_presets,
            // The Python analysis sidecar (ADR 0001). These are the only commands that can be
            // slow, and the only ones that read a file the app did not write itself.
            sidecar::list_replays,
            sidecar::scan_library,
            sidecar::analyze_replay,
            sidecar::replay_build_order
        ])
        // The shortcut is registered from inside `setup` rather than by the builder so a
        // failure is recoverable. On Wayland registration is expected to fail — the plugin
        // is X11-only — and an error returned from `setup` would take the whole app down
        // with it, leaving the player with no window to toggle the overlay from either.
        .setup(|app| {
            // Register the toggle key on a background thread, not here. On Wayland the
            // portal's `BindShortcuts` opens a compositor approval dialog that the user may
            // take any amount of time to answer, and blocking `setup` would mean the main
            // window never appeared until they did. The thread tries the portal first and
            // falls back to the X11 plugin (see `register_overlay_shortcut`).
            let handle = app.handle().clone();
            std::thread::spawn(move || register_overlay_shortcut(&handle));

            // Warm the hidden webview, and reveal it instead when the app is not meant to
            // start hidden. See `warm_or_reveal_main_window` for why both halves happen
            // together and why the show half is not optional.
            warm_or_reveal_main_window(app.handle().clone());

            // Watch the game so the popup can appear when a match starts. In Rust rather than
            // the frontend because the window is hidden between games and a hidden WebKitGTK
            // widget has its page timers suspended — see `game_watch`.
            game_watch::watch_for_game_start(app.handle().clone());

            // The tray icon, which is how a resident app is reached when its window is hidden.
            // Without it the app could be running invisibly with no way to quit it short of a
            // task manager.
            if let Err(err) = build_tray(app.handle()) {
                eprintln!("could not create the tray icon ({err})");
            }

            // Development affordance: start in overlay mode so its real compositor placement
            // can be inspected without pressing the hotkey. Reading the stack back from the X
            // server is the only way to check z-order and position. Not a user-facing setting.
            // Any value other than `0` also means "show it", so setting the variable to a
            // path or `true` still works; only an explicit `0` disables it.
            if std::env::var_os("SC2SV_START_WITH_OVERLAY")
                .is_some_and(|value| value != "0")
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    // `Shown` because the point of the affordance is to see the overlay.
                    if let Err(err) =
                        apply_window_mode(&handle, WindowMode::Overlay, Visibility::Shown)
                    {
                        eprintln!("could not switch to overlay mode at startup ({err})");
                    }
                });
            }

            arm_main_window_watchdog(app.handle().clone());
            Ok(())
        })
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    // Act on press only: a handler that also fired on release would toggle
                    // the overlay twice per keystroke and net out to no visible change.
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let _ = toggle_window(app.clone());
                    }
                })
                .build(),
        )
        .run(tauri::generate_context!())
        .expect("error while running sc2-science-vessel");
}

/// Registers the overlay key, preferring the Wayland portal over the X11 grab.
///
/// The key shows and hides the app window; it does not change the window's mode. See
/// `toggle_window` for why those are different things.
///
/// The two mechanisms must not both be active. `tauri-plugin-global-shortcut`'s X11 grab
/// is app-scoped on Wayland, so it still fires while the app itself has focus; if the
/// portal also held the key, one keypress would fire both and appear to do nothing. So this
/// tries exactly one of them, portal first.
///
/// The portal is preferred because its grabs are compositor-level and therefore keep
/// working while the game holds focus — the entire point of the key. It is not guaranteed:
/// `CreateSession` is refused outright when the portal cannot resolve an application id,
/// which happens for a binary launched from a shell without a matching `.desktop` file.
/// The X11 plugin is the fallback, and it is also the only path on Windows and macOS.
///
/// @param app The running app handle.
#[cfg(target_os = "linux")]
fn register_overlay_shortcut(app: &tauri::AppHandle) {
    // The portal conversation has to happen off the main thread: `BindShortcuts` waits for
    // the user to answer the compositor's dialog, which can take arbitrarily long, and the
    // main thread has a window to show. The X11 fallback is decided *inside* that thread so
    // it is only taken when the portal actually refused — registering both would make one
    // keypress toggle the overlay twice.
    let app = app.clone();
    std::thread::spawn(move || {
        if !bind_overlay_shortcut_via_portal(&app) {
            register_overlay_shortcut_via_x11(&app);
        }
    });
}

/// Registers the overlay key through `tauri-plugin-global-shortcut` on non-Linux platforms.
///
/// Windows and macOS have no XDG portal, so the plugin *is* the global-shortcut mechanism
/// there rather than a fallback for one.
///
/// @param app The running app handle.
#[cfg(not(target_os = "linux"))]
fn register_overlay_shortcut(app: &tauri::AppHandle) {
    register_overlay_shortcut_via_x11(app);
}

/// Registers the overlay key through `tauri-plugin-global-shortcut`.
///
/// This is the X11 path on Linux, and the only path on Windows and macOS. It is separate
/// from `register_overlay_shortcut` because the Wayland portal runs on its own thread and
/// reaches here as a fallback when it refuses the binding.
///
/// @param app The running app handle.
fn register_overlay_shortcut_via_x11(app: &tauri::AppHandle) {
    let loaded = Settings::load(app);

    // Register from the stored setting, not a constant, so a rebind survives a restart.
    // `register_overlay_shortcut` walks a fallback list when the stored key is taken, so a
    // fresh install still gets a working toggle key.
    match settings::register_overlay_shortcut(app, &loaded.overlay_shortcut) {
        Ok(registered) => {
            if registered != loaded.overlay_shortcut {
                eprintln!(
                    "overlay shortcut {} was unavailable; registered {registered} instead",
                    loaded.overlay_shortcut
                );
                // Persist the substitution so the settings page shows the key that actually
                // works rather than the one that failed to register.
                let mut updated = loaded.clone();
                updated.overlay_shortcut = registered;
                if let Err(err) = updated.save(app) {
                    eprintln!("could not persist the fallback shortcut ({err})");
                }
            }
        }
        Err(err) => eprintln!(
            "no global shortcut could be registered ({err}); \
			 the overlay can still be toggled from the app window. \
			 The XDG GlobalShortcuts portal is the Wayland path — see docs/adr/0007"
        ),
    }
}

/// Binds the overlay toggle key to the XDG `GlobalShortcuts` portal.
///
/// Drives the whole portal conversation on the calling thread and then, if the binding
/// succeeded, stays there to relay `Activated` signals for the rest of the process's life.
/// That is why it never returns `true` without a live activation loop behind it: a bound
/// shortcut with nobody listening would be worse than no bound shortcut, because the
/// compositor would swallow the key and nothing would happen.
///
/// Errors are reported and turned into `false` rather than propagated: every failure here
/// is either "this compositor does not implement the portal" or "the portal refused this
/// app", and both have the same response — fall back to the X11 plugin.
///
/// @param app The running app handle, used to toggle the overlay when the key is pressed.
/// @returns `true` if the portal took the binding and an activation loop is running.
#[cfg(target_os = "linux")]
fn bind_overlay_shortcut_via_portal(app: &tauri::AppHandle) -> bool {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_lite::StreamExt;

    let loaded = Settings::load(app);
    let trigger = match portal_shortcut::to_portal_trigger(&loaded.overlay_shortcut) {
        Some(trigger) => trigger,
        None => {
            eprintln!(
                "overlay shortcut {:?} names no key, so the portal cannot bind it",
                loaded.overlay_shortcut
            );
            return false;
        }
    };

    portal_shortcut::block_on(async move {
        let shortcuts = match GlobalShortcuts::new().await {
            Ok(shortcuts) => shortcuts,
            Err(err) => {
                eprintln!("global shortcut portal unavailable ({err})");
                return false;
            }
        };

        // `CreateSessionOptions::default()` fills in the two tokens the portal requires.
        // Omitting them is not a soft failure: xdg-desktop-portal 1.22.1 aborts on an
        // assertion when `CreateSession` is called without them.
        let session = match shortcuts.create_session(Default::default()).await {
            Ok(session) => session,
            Err(err) => {
                eprintln!(
                    "global shortcut portal declined a session ({err}); \
					 falling back to the X11 shortcut plugin"
                );
                return false;
            }
        };

        let shortcut = NewShortcut::new(
            portal_shortcut::OVERLAY_SHORTCUT_ID,
            portal_shortcut::OVERLAY_SHORTCUT_DESCRIPTION,
        )
        .preferred_trigger(trigger.as_str());

        // Opens the compositor's approval dialog. The user may decline, in which case the
        // portal still records the binding with no key so it does not ask again.
        let request = match shortcuts
            .bind_shortcuts(&session, &[shortcut], None, Default::default())
            .await
        {
            Ok(request) => request,
            Err(err) => {
                eprintln!("could not bind the overlay shortcut via the portal ({err})");
                return false;
            }
        };

        // This blocks until the user answers the dialog, which is why the whole function
        // runs on its own thread.
        let bound = match request.response() {
            Ok(response) => {
                let bound = response
                    .shortcuts()
                    .iter()
                    .map(|shortcut| {
                        format!("  {} -> {}", shortcut.id(), shortcut.trigger_description())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                // A bound shortcut with an EMPTY trigger means the user dismissed the
                // compositor's dialog. KDE registers the id anyway, with no key, precisely so
                // it will not ask again — so this is not a transient state that resolves
                // itself, and reporting success here would leave the player pressing a dead
                // key for the life of the install. Confirmed on this machine: the binding
                // persisted in `~/.config/kglobalshortcutsrc` as
                // `toggle-overlay=none,none,Show or hide the build order overlay`.
                let has_trigger = response
                    .shortcuts()
                    .iter()
                    .any(|shortcut| !shortcut.trigger_description().trim().is_empty());
                println!("overlay shortcut registered through the XDG portal\n{bound}");
                if !has_trigger {
                    eprintln!(
                        "the portal bound no key for the overlay shortcut (the approval dialog \
                         was probably dismissed); the desktop will not ask again for this id, so \
                         the X11 fallback is being used instead. To bind it through the portal, \
                         set a key in System Settings > Shortcuts > Konsole (the portal resolves \
                         this app's identity from the terminal it was launched from until it is \
                         installed), then restart the app"
                    );
                }
                has_trigger
            }
            Err(err) => {
                eprintln!("the portal rejected the overlay shortcut ({err})");
                return false;
            }
        };

        // The binding exists but names no key, which cannot toggle anything. Reached when the
        // player dismissed the dialog, and handled by returning `false` so the caller tries the
        // X11 grab rather than leaving the app with no toggle key at all. The session and its
        // binding are dropped with this function; KDE keeps the keyless id registered, so the
        // fallback is the only thing that can work until the player sets one in System
        // Settings.
        if !bound {
            return false;
        }

        // Relay activations for the life of the process. Returning here would drop the
        // stream and, with it, the binding.
        let mut activated = match shortcuts.receive_activated().await {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("could not listen for overlay shortcut activations ({err})");
                return true;
            }
        };
        while let Some(event) = activated.next().await {
            if event.shortcut_id() == portal_shortcut::OVERLAY_SHORTCUT_ID {
                if let Err(err) = toggle_window(app.clone()) {
                    eprintln!("could not toggle the window ({err})");
                }
            }
        }
        true
    })
}

/// Whether the stacking watcher thread is alive.
///
/// Only meaningful on Linux; declared unconditionally so the flag's lifetime is obvious and
/// there is one place to look for who owns the watcher.
static OVERLAY_WATCH_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Window size in analysis mode, in logical pixels.
///
/// Restored on every switch back, because the window was resized to overlay dimensions and a
/// player returning from a game should get the window they left, not a 380x300 stub.
const ANALYSIS_WIDTH: f64 = 1280.0;
/// Window height in analysis mode, in logical pixels.
const ANALYSIS_HEIGHT: f64 = 820.0;

/// Smallest the analysis window may be dragged to, in logical pixels.
///
/// A floor is needed because the analysis pages are dense — a table of build steps, a chart
/// with axes, a sidebar of metrics — and below roughly this size the layout stops being a
/// smaller version of itself and starts being unusable. Matches the `minWidth`/`minHeight`
/// declared in `tauri.conf.json`.
const ANALYSIS_MIN_WIDTH: f64 = 900.0;
/// Smallest analysis height, in logical pixels. See [`ANALYSIS_MIN_WIDTH`].
const ANALYSIS_MIN_HEIGHT: f64 = 600.0;

/// Largest the analysis window may be dragged to, in logical pixels.
///
/// Generous rather than restrictive: 4K is the reference machine's resolution and a window
/// spanning it is a legitimate thing to want. The bound exists only so a corrupt or hostile
/// resize request cannot produce a window larger than any desktop, which the user could not
/// drag back into view. Matches `maxWidth`/`maxHeight` in `tauri.conf.json`.
const ANALYSIS_MAX_WIDTH: f64 = 7680.0;
/// Largest analysis height, in logical pixels. See [`ANALYSIS_MAX_WIDTH`].
const ANALYSIS_MAX_HEIGHT: f64 = 4320.0;

/// Default overlay width, in logical pixels.
const OVERLAY_WIDTH: f64 = 380.0;
/// Default overlay height, in logical pixels.
const OVERLAY_HEIGHT: f64 = 300.0;
/// Distance from the game window's top-left corner to the overlay's, in logical pixels.
const OVERLAY_INSET: i32 = 24;

#[cfg(test)]
mod window_visibility_tests {
    use super::{should_reveal, Visibility};

    /// A player pressing the hotkey is asking to *see* the app, so the window must be shown
    /// even though it is currently hidden — this is the whole point of the hotkey.
    #[test]
    fn a_hotkey_show_reveals_a_hidden_window() {
        assert!(should_reveal(Visibility::Shown, false));
        assert!(should_reveal(Visibility::Shown, true));
    }

    /// The regression this rule exists to prevent: the game ends while the app is hidden, the
    /// mode changes automatically, and the window must stay hidden because the player hid it.
    #[test]
    fn an_automatic_mode_change_leaves_a_hidden_window_hidden() {
        assert!(!should_reveal(Visibility::AsIs, false));
    }

    /// An automatic mode change on an already-visible window must still run the show path:
    /// that call is what re-establishes the overlay's click-through and stacking flags, which
    /// do not survive a hide/show cycle.
    #[test]
    fn an_automatic_mode_change_keeps_a_visible_window_visible() {
        assert!(should_reveal(Visibility::AsIs, true));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod overlay_stacking_tests {
    use super::desired_keep_above;

    /// The ordinary case: the game is on screen and nobody is fullscreen, so the overlay takes
    /// the top and is then restacked to sit exactly one window up.
    #[test]
    fn the_overlay_claims_the_top_over_a_windowed_game() {
        assert!(desired_keep_above(false, true));
    }

    /// A fullscreen client is on the compositor's `ActiveLayer`, above anything keep-above can
    /// reach, so claiming the top would only misreport the situation. Dropping it means the
    /// overlay is honestly behind the game and can be raised.
    #[test]
    fn the_overlay_yields_to_another_clients_fullscreen() {
        assert!(!desired_keep_above(true, true));
    }

    /// No game window at all: the overlay must not float over unrelated windows while the
    /// player is doing something else.
    #[test]
    fn the_overlay_releases_the_top_when_the_game_is_gone() {
        assert!(!desired_keep_above(false, false));
        assert!(!desired_keep_above(true, false));
    }
}

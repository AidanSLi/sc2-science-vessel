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
//! The hotkey *does* pick a mode, but only the mode to reveal: a press asks to see the app, and
//! what it should see depends on whether a match is being played (ADR 0016). That is a
//! different question from "has the player deliberately hidden this window", which no client
//! reading can answer.
//!
//! Two constraints from the feasibility study shape this file and must not be broken:
//!
//! - The app never reads the game process's memory, and never sends input to the game.
//!   The only sanctioned live input is the SC2 Client API on `localhost:6119`, which both the
//!   frontend (`src/lib/client-api.ts`) and `game_watch` read (ADR 0004). Process inspection
//!   exists only to notice that the game started or exited; it reads `/proc` names and never
//!   touches the game. This is what keeps the overlay on the benign side of Blizzard's
//!   discretionary EULA clause.
//! - The window is a plain always-on-top, click-through toplevel for now, positioned by
//!   restacking over the game's X11 window (ADR 0008). KDE exposes `zwlr_layer_shell_v1`, so a
//!   real compositor-level overlay surface is reachable and is the planned upgrade (ADR 0003).

use std::path::Path;
use tauri::{Emitter, Manager, WebviewWindow};

#[cfg(target_os = "linux")]
mod portal_shortcut;
pub mod autostart;
mod control;
pub mod gui_control;
mod game_watch;
mod notify;
mod platform;
mod settings;
mod sidecar;
#[cfg(target_os = "linux")]
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
    let target = requested_mode(&mode);
    // `AsIs`, not `Shown`: this command reports the mode the frontend should render, and
    // deciding to reveal the window is the hotkey's job. A player who hides the app and then
    // has the mode change underneath them must not have the window pop back up.
    apply_window_mode(&app, target, Visibility::AsIs)?;
    Ok(target)
}

/// Applies a mode the app chose for itself *and* reveals the window.
///
/// Distinct from [`set_window_mode_command`], which deliberately leaves visibility alone. The
/// difference is who asked: a player pressing Escape wants the view to change without the
/// window reappearing, whereas the match boundary is exactly the moment the user asked the app
/// to put itself on screen — ``upon match start, the app is now visible and in its build order
/// state'' and ``upon match end, the app now transitions to its analysis state''.
///
/// The frontend only calls this when the mode actually changed, so a player who hid the window
/// mid-match is not overruled by every subsequent poll: hiding does not change the mode, so
/// nothing calls back in.
///
/// @param app The running app handle.
/// @param mode The mode to switch to.
/// @returns The mode now in effect.
#[tauri::command]
fn reveal_window_mode_command(app: tauri::AppHandle, mode: String) -> tauri::Result<WindowMode> {
    let target = requested_mode(&mode);
    apply_window_mode(&app, target, Visibility::Shown)?;
    Ok(target)
}

/// The mode `SC2SV_PIN_OVERLAY` is holding the window in, when it is switched on.
///
/// The pin is a development affordance: it exists to keep the overlay on screen for inspection
/// (and for screenshot comparisons) when no match is running, so it must outrank every other
/// source of a mode — the frontend's request, the hotkey's reading of the client, the tray.
/// One function rather than a guard duplicated at each entry point, because a fourth caller
/// added later would otherwise silently ignore it.
///
/// @returns `Some(Overlay)` while the pin is on, and `None` when the pin is off.
fn pinned_mode() -> Option<WindowMode> {
    if PINNED_OVERLAY.load(std::sync::atomic::Ordering::SeqCst) {
        Some(WindowMode::Overlay)
    } else {
        None
    }
}

/// Interprets a mode name arriving over IPC, honouring the pinned overlay.
///
/// Anything unrecognised is analysis: that is the mode in which the player can still see and
/// click the app, so a bad or crafted payload cannot leave them holding a click-through panel.
///
/// @param mode The raw mode name from the frontend.
/// @returns The mode to apply.
fn requested_mode(mode: &str) -> WindowMode {
    pinned_mode().unwrap_or(match mode {
        "overlay" => WindowMode::Overlay,
        _ => WindowMode::Analysis,
    })
}

/// Reads the window's current mode, so the frontend can render the matching view.
///
/// The frontend cannot infer this: the hotkey and the tray change the mode without any frontend
/// involvement, so the mode has to be asked for rather than derived from a local variable.
///
/// @returns The mode the window is in.
#[tauri::command]
fn get_window_mode() -> WindowMode {
    window_mode()
}

/// Reports whether this machine can anchor the overlay to the game window.
///
/// Exists because the answer is not derivable from the platform name: a Linux session may be
/// X11 (supported) or Wayland-only (not), and a user on the unsupported case otherwise has no way
/// to tell "the overlay is broken" from "this display server cannot do this". The settings page
/// shows it, so the limitation is visible before the player notices the overlay drifting rather
/// than after.
///
/// @returns `true` when the overlay can track the game's window.
#[tauri::command]
fn overlay_supported_command() -> bool {
    platform::overlay_supported()
}

/// Reports whether animating the ambient background would be cheap on this machine.
///
/// Exists because the same CSS costs nothing on one platform and a whole CPU core on another.
/// Linux runs WebKitGTK, which this app is forced onto a *software* compositor
/// (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, ADR 0005 — the NVIDIA driver refuses the buffer WebKit
/// asks for, ADR 0022). Under it a continuously animating element costs 100% of a core whatever
/// its size, because every frame repaints the whole window on the CPU (ADR 0021). Windows runs
/// WebView2 over Chromium's compositor, where transform and opacity animations run on the GPU.
///
/// The frontend asks rather than guessing from `navigator.userAgent`, so the rule lives next to
/// the platform code it depends on and can be changed in one place when a Linux machine with a
/// working GPU path is supported.
///
/// @returns `true` when the ambient background may animate.
#[tauri::command]
fn animations_supported_command() -> bool {
    cfg!(windows)
}

/// Decides whether the hotkey should *hide* rather than reveal, from what the player can see.
///
/// The hotkey is a toggle, and the honest question it asks is "is the app in front of me?" —
/// not "is the window mapped?". Those differ exactly where the user reported the key doing
/// nothing: with StarCraft II in fullscreen and focused, the app window can still be *mapped*
/// while sitting underneath the game, because a managed window cannot out-rank an active
/// fullscreen client (docs/adr/0015 records the measurements: a keep-above managed window is
/// 0 % visible over an active fullscreen client). `is_visible()` then answers `true` for a
/// window the player cannot see, so the press took the hide branch and the app disappeared from
/// a place the player was not looking — indistinguishable from a dead key. Pressing again then
/// revealed it, so the key "worked" only on alternate presses.
///
/// So a mapped window is only treated as visible when the window manager also reports it as the
/// active window. When it is mapped but not active, the press reveals instead, which is what the
/// player is asking for, and revealing is what raises it above the game.
///
/// The mode has to be part of the decision, because it decides whether "active" is a meaningful
/// question at all. **Overlay mode is click-through and refuses focus by design** (ADR 0018:
/// `refuse_focus` sets `set_accept_focus(false)` and `set_focus_on_map(false)`, so the game keeps
/// focus while the guide is up). An overlay can therefore never be the active window, and asking
/// whether it is active answers a question with only one possible answer. Treating that answer as
/// "not visible" made the key a reveal-only key in matches: measured as `is_visible=true
/// is_active=false hiding=false mode=Overlay`, after which `reveal_for_hotkey` flipped the window
/// to unmanaged — the frame or so of blankness the player saw — and the still-running stacking
/// watcher put it straight back. The overlay vanishing for a moment and returning is that cycle.
///
/// In overlay mode the only question that matters is whether the window is on screen, which is
/// exactly what `mapped` already answers, so the press toggles. In analysis mode the window is an
/// ordinary focusable window and the mapped-but-buried case is real, so `active` still decides.
///
/// @param mode The mode the window is currently in.
/// @param mapped Whether the toolkit reports the window as visible.
/// @param active Whether the window manager reports the window as active.
/// @returns `true` when the press should hide the window.
fn hotkey_should_hide(mode: WindowMode, mapped: bool, active: bool) -> bool {
    if !mapped {
        return false;
    }
    match mode {
        // A click-through panel cannot be active, so its on-screen state is the whole story.
        WindowMode::Overlay => true,
        WindowMode::Analysis => active,
    }
}

/// Shows or hides the app window, choosing the mode it comes up in.
///
/// **This is what the global hotkey does.** Pressing it while the window is hidden reveals the
/// window in the mode the player's situation calls for (see [`reveal_for_hotkey`]); pressing it
/// while the window is visible hides it.
///
/// The mode used to be left alone here, on the theory that the hotkey was a pure visibility
/// control. That was wrong in the case the user named: a player in SC2's menus pressed the key
/// and got whatever mode the last match left behind — usually the click-through guide, which is
/// a 380-pixel panel with no pointer events, over a menu where they wanted to read their
/// matches. The analysis window is what that press is asking for, and only the in-match case
/// wants the guide.
///
/// @param app The running app handle.
/// @returns `true` if the window is now visible.
#[tauri::command]
fn toggle_window(app: tauri::AppHandle) -> tauri::Result<bool> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(false);
    };

    let mapped = window.is_visible().unwrap_or(false);
    // Ask the window manager, not just the toolkit: a mapped window underneath an active
    // fullscreen game is not something the player can see, and treating it as visible made the
    // hotkey's first press hide a window nobody was looking at. See `hotkey_should_hide`.
    let active = if mapped {
        platform::is_active_window(&window)
    } else {
        false
    };
    let visible = hotkey_should_hide(window_mode(), mapped, active);
    if std::env::var_os("SC2SV_DEBUG_TOGGLE").is_some() {
        eprintln!(
            "toggle: is_visible={mapped} is_active={active} hiding={visible} mode={:?}",
            window_mode()
        );
    }
    if visible {
        // Stop the stacking watcher first, then take the lock: a tick already inside
        // `claim_unmanaged` would otherwise finish its hide/show flip after the hide below and
        // put the overlay back on screen, which is exactly the "the hotkey only works one way"
        // symptom this ordering exists to prevent.
        OVERLAY_WATCH_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
        let _guard = OVERLAY_STACK_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        // Hide first, and tell the release not to re-show. Both parts are needed. The release
        // flips the override-redirect attribute by hiding and re-showing the window, and by
        // design it preserves "was visible" across that flip so a stacking change never alters
        // whether the player can see the app. This path wants the opposite, so it passes
        // `false` — measured before that parameter existed: three consecutive presses each
        // reported `is_visible=true` because the flip kept putting the overlay back.
        //
        // The hide is left merely queued (rather than pumped through the GTK loop) because GTK
        // may only be driven from the main thread and `toggle_window` runs on the global-shortcut
        // thread. `is_visible` reads the queued value, which is exactly what the release needs to
        // see to skip its own re-show.
        window.hide()?;
        release_unmanaged_and_visibility(&window, false);
        Ok(false)
    } else {
        reveal_for_hotkey(&app)?;
        Ok(true)
    }
}

/// Reveals the window in the mode the game's own client says the player is in.
///
/// Distinct from [`show_window`] on purpose, and the difference is who asked. This is the
/// hotkey's reveal: a player pressing the key is asking to *see* the app, and which of the two
/// modes that means is a fact about the game, not about the last thing the app did. The rule is
/// [`game_watch::hotkey_mode_now`] — a match being played means the guide, a running game with
/// any screen open (or no game at all) means the analysis window.
///
/// It is decided here rather than pushed up from the frontend because a hidden WebKitGTK webview
/// has its page timers suspended: at the exact moment the hotkey fires, the page that would have
/// known the screen list is not running. See `game_watch`'s module docs.
///
/// A client that answers `/game` but not `/ui` leaves the stored mode in place (that is the
/// `None` case), and the development affordance that pins the overlay wins over everything, so
/// the overlay can still be inspected outside a live match.
///
/// @param app The running app handle.
/// @returns `Err` only if the window could not be put into the chosen mode.
fn reveal_for_hotkey(app: &tauri::AppHandle) -> tauri::Result<()> {
    let mode = pinned_mode()
        .or_else(game_watch::hotkey_mode_now)
        .unwrap_or_else(window_mode);
    if std::env::var_os("SC2SV_DEBUG_TOGGLE").is_some() {
        eprintln!("toggle: revealing in mode={mode:?}");
    }
    let result = apply_window_mode(app, mode, Visibility::Shown);
    // Raise *after* the mode has been applied, and on the main thread, because every geometry
    // call in `apply_window_mode` (`show`, `set_size`, `center`) is posted to tao's event-loop
    // channel rather than executed inline. Those posted requests are handled in arrival order, so
    // a raise issued from inside the function is processed first and then undone by the resize
    // that follows it. Measured before this move: a press with the game focused left the app at
    // stacking index 3 below the game's 4, and `_NET_ACTIVE_WINDOW` still named the game.
    //
    // Reaching the main thread is required, not merely tidy: `set_focus` would otherwise be the
    // only way to raise the window, and it uses source indication 1, which KWin refuses for a
    // client the user has not just clicked.
    let handle = app.clone();
    let raised = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(MAIN_WINDOW_LABEL) {
            platform::raise_and_activate(&window);
        }
    });
    if let Err(err) = raised {
        eprintln!("could not raise the window after the hotkey ({err})");
    }
    result
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
    let start_hidden = start_hidden_for_this_launch(&app);
    std::thread::spawn(move || {
        let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        // Map it once so the webview is painted and its event delivery is known to work.
        let _ = window.show();
        let _ = window.hide();

        if !start_hidden {
            let _ = window.show();
            // A view that was transparent from creation can finish its first composite with an
            // empty backing store, leaving the window fully see-through until something later
            // forces a repaint. Measured on the analysis view: one launch in five started at
            // 99.7% alpha-0 and stayed that way, because nothing repainted until the player
            // moved or resized the window. The frontend nudges a repaint on resize and on the
            // clock, but neither happens before the window is on screen.
            //
            // Re-asserting the webview's own background once it is mapped is the cheap fix: the
            // setter asks WebKit to clear the surface to an opaque colour, which is the paint
            // the empty state was missing. It is the same call and the same colour as
            // `apply_window_mode`, so it cannot disagree with it, and it is harmless when the
            // first composite already succeeded.
            if window_mode() == WindowMode::Analysis {
                let webview: &tauri::Webview<tauri::Wry> = window.as_ref();
                if let Err(err) = webview.set_background_color(Some(ANALYSIS_BACKGROUND)) {
                    eprintln!("could not repaint the starting window ({err})");
                }
            }
        }
    });
}

/// Listens for requests from the watcher to reveal this window.
///
/// The watcher spawns the GUI hidden when StarCraft II starts, because a game starting is not a
/// match starting (see `watcher`). Anything that later decides the window should be visible —
/// the player clicking the app icon again, or the watcher itself — reaches the GUI through
/// this, because the GUI is a separate process and cannot be told directly.
///
/// The listener makes the window visible in the mode it already has. It deliberately does not
/// pick a mode: whether the window is a guide or an analysis view follows the client API's
/// screen list (ADR 0010), and a second process able to override that would be a second source
/// of truth for the same fact.
///
/// The hotkey path differs — see [`reveal_for_hotkey`], which does choose — because a click on
/// the app icon is "bring back the window I left where it was", whereas the hotkey is the
/// player asking for the app while the game is on screen, and the answer to that depends on
/// whether they are mid-match. Both still read the same single client reading.
///
/// @param app The running app handle.
fn start_gui_listener(app: tauri::AppHandle) {
    gui_control::start(move || {
        if let Err(err) = show_window(&app) {
            eprintln!("could not show the window for a watcher request ({err})");
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
            // Before the show, not after it: `set_accept_focus(false)` only influences whether a
            // window takes focus *when it is mapped*, so setting it afterwards leaves the map
            // itself having stolen activation from the game. Measured: with the call after
            // `show()` the overlay became `_NET_ACTIVE_WINDOW` and the fullscreen game was
            // demoted, which the stacking watcher then read as "the game is not active".
            platform::refuse_focus(&window);
            // Back to a fully transparent web view, so the guide shows the game through its own
            // text and nothing else. Set on every overlay entry rather than only when leaving
            // analysis: a mode can be entered without the other having been left (the startup
            // path applies the stored mode directly), and a guide that came up with an opaque
            // black backdrop would hide the game it is supposed to float over.
            // Note the argument: `Some(Color(0, 0, 0, 0))`, not `None`. `None` does *not* mean
            // "transparent" — tauri-runtime-wry 2.12.1 maps it to opaque white at
            // `src/lib.rs:3789` (`color.map(Into::into).unwrap_or((255, 255, 255, 255))`), and
            // measured on this build that painted the guide onto a 94.3 % `rgba(254,254,254,255)`
            // panel that hid the game completely. A zero-alpha colour is the value that actually
            // clears the surface.
            let webview: &tauri::Webview<tauri::Wry> = window.as_ref();
            if let Err(err) = webview.set_background_color(Some(TRANSPARENT_BACKGROUND)) {
                eprintln!("could not make the overlay's background transparent ({err})");
            }
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
            // Deliberately NOT `set_always_on_top(true)`. The stacking watcher owns that flag: it
            // is the only code that knows whether the game is windowed or fullscreen, and it
            // tracks the flag's value to avoid re-sending it every tick. Setting it here would
            // desynchronise that bookkeeping — measured: the watcher believed the flag was off
            // while KWin reported `keepAbove=true`, so it never cleared it and the overlay stayed
            // in `AboveLayer` above the very window the player had raised over the game.
            window.set_ignore_cursor_events(true)?;
            // Only watch stacking for a visible overlay: the watcher's whole job is to express
            // "above the game", and a hidden window has nothing to stack. On a platform with no
            // window-tracking API the watcher starts, reports `Unsupported` once, and exits.
            if should_show {
                // A window left override-redirect by a previous overlay session must be managed
                // again before the watcher measures anything: an unmanaged window is not
                // tracked by the window manager, so it cannot be restacked or resized and
                // would look like a frozen rectangle over the game.
                release_unmanaged(&window);
                watch_overlay_stacking(window.clone());
            }
        }
        WindowMode::Analysis => {
            // Stop the stacking watcher before anything else: it re-asserts the overlay's
            // stacking and keep-above state several times a second, and would fight the mode
            // change if it kept running.
            OVERLAY_WATCH_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
            // The overlay may have gone override-redirect to sit above an active fullscreen
            // game. In analysis mode the window must be an ordinary managed window again: an
            // unmanaged one gets no decorations, no taskbar entry, no focus and no resizing,
            // so it could not be clicked or typed into at all.
            release_unmanaged(&window);
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
            // The inverse of the overlay's refusal, and mandatory rather than cosmetic: the flag
            // persists on the toolkit's side, so leaving it set would make analysis mode a window
            // that neither raises when clicked nor accepts a keystroke. It is set before the show
            // for the same reason the refusal is — the flag is consulted when the window is
            // mapped.
            platform::allow_focus(&window);
            window.set_always_on_top(false)?;
            // Decorations stay OFF in analysis mode too, and the window is dragged and closed
            // from the app's own title bar instead. A native title bar is drawn by the toolkit
            // in the desktop's own theme, so it cannot follow a race theme: the user asked for a
            // title bar flush with the app content, and a GTK headerbar strip above a Terran
            // panel is exactly the seam that request was about. The chrome is part of the page,
            // and `platform::allow_focus` above is what keeps it interactive.
            //
            // The cost is that a frameless window has no toolkit-provided resize border, so the
            // page supplies its own (see `TitleBar.svelte`), calling `start_resize_dragging`.
            window.set_decorations(false)?;
            // A shadow survives the lost decoration on platforms that draw it as its own frame;
            // asking explicitly keeps a frameless window separable from the desktop behind it.
            window.set_shadow(true)?;
            window.set_resizable(true)?;
            window.set_skip_taskbar(false)?;
            // Give the web view an opaque background. The page's own `.app-surface` already
            // paints the same colour (ADR 0017), so this is the second of two defences, and it
            // is deliberately the *same* colour: the two agree, so there is no seam while the
            // page is still laying out.
            //
            // Both are wanted because the two fail in different circumstances. `.app-surface`
            // needs the page to have been mounted; a view created transparent and shown before
            // its first composite has nothing underneath it until then. Measured without this
            // call, three launches in a row painted 100% — so the CSS alone is sufficient in
            // practice — but this costs one call and removes the window in which the desktop can
            // show through.
            //
            // This is `Webview::set_background_color`, deliberately not
            // `WebviewWindow::set_background_color`: the latter also sets it on the *window*,
            // which goes through GTK CSS and paints over the page instead of behind it. Measured
            // there: asking for `(10,13,18)` produced a 98.4 % white window, and `(0,8,255)`
            // produced `(0,255,255)`. The web-view-only setter round-trips exactly.
            let webview: &tauri::Webview<tauri::Wry> = window.as_ref();
            if let Err(err) = webview.set_background_color(Some(ANALYSIS_BACKGROUND)) {
                // Not fatal: the page's own surface still paints. Worth a line, not a failed mode
                // change.
                eprintln!("could not set the analysis background colour ({err})");
            }
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
            // Only raise and focus when the player asked for the window. An automatic mode change
            // must never pull focus away from a running game.
            //
            // `set_focus` alone is not enough, and the gap is exactly the reported bug: it asks
            // the window manager to activate with source indication 1 ("ordinary application"),
            // which KWin refuses for a client the user has not just interacted with. With a
            // fullscreen game focused and the app already mapped, the request left the app at
            // stacking index 3 below the game's 4 and the game active — measured. The EWMH
            // request `platform::raise_and_activate` sends uses source indication 2 ("direct user
            // action"), which the same measurement showed honoured immediately. The hotkey is a
            // direct user action, so that is the honest value.
            //
            // Both are attempted: the EWMH path is Linux-only and answers `false` elsewhere,
            // where `set_focus` is the correct and sufficient call.
            if visibility == Visibility::Shown {
                platform::raise_and_activate(&window);
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
fn reposition_for_overlay(window: &WebviewWindow, game: Option<platform::Rect>) {
    let (x, y) = platform::overlay_position(window, game);
    if let Err(err) = window.set_position(tauri::LogicalPosition::new(x, y)) {
        eprintln!("could not move the overlay ({err})");
    }
}

/// Whether the overlay should be claiming the always-on-top state this tick.
///
/// A pure function so the decision is testable: the two behaviours that matter — yielding to a
/// fullscreen client, and dropping the claim when the game is gone — are otherwise observable
/// only by watching a window manager. Restating the rule: the claim holds only while the game is
/// found and nobody else is fullscreen.
///
/// The platform matters twice. [`platform::CLAIM_ALWAYS_ON_TOP`] is `false` on Windows, where the
/// claim is too strong to express the requirement, so the answer is `false` there regardless of
/// what was found — the restacking is what places the overlay and it does not need the claim.
///
/// @param other_fullscreen Whether another client holds an active fullscreen window.
/// @param game_anchored Whether the game window was found and restacked against.
/// @returns Whether the window should be always-on-top.
fn desired_keep_above(other_fullscreen: bool, game_anchored: bool) -> bool {
    platform::CLAIM_ALWAYS_ON_TOP && !other_fullscreen && game_anchored
}

/// Applies [`desired_keep_above`] to the window, touching it only on a transition.
///
/// The flag is a window property, so setting it on every poll would send a request per tick for
/// a value that is almost always unchanged. The position is different: the player may drag or
/// resize the game at any time, and the overlay is meant to sit in the game's top-left corner, so
/// the move is attempted every poll.
///
/// @param window The overlay window.
/// @param other_fullscreen Whether another client holds an active fullscreen window.
/// @param game_anchored Whether the game window was found and restacked against.
/// @param keep_above The watcher's current belief about the window's always-on-top state.
/// @returns The state the caller should carry into the next poll.
fn apply_overlay_stacking(
    window: &WebviewWindow,
    other_fullscreen: bool,
    game_active: bool,
    game_anchored: bool,
    keep_above: bool,
) -> bool {
    // A fullscreen client can only be out-ranked by becoming unmanaged. Doing that while the
    // game is windowed would be worse than useless: an unmanaged window cannot be restacked
    // behind a window the player raises, which is the behaviour that makes the windowed case
    // feel right. So the claim is tied to the fullscreen state rather than taken once.
    // `other_fullscreen` alone is not enough: it stays true while the game is fullscreen even
    // after the player raises something over it, and becoming unmanaged then would cover the
    // window they just raised. Only an *active* fullscreen game needs out-ranking.
    if other_fullscreen && game_active && game_anchored && platform::can_outrank_fullscreen() {
        claim_unmanaged(window);
    } else if release_unmanaged(window) && game_anchored {
        // The release re-showed the window, which raised it to the top of the stack. Restacking it
        // is not optional: the whole point of releasing is to let the overlay go *behind* whatever
        // the player raised over the game, and an overlay left at the top would cover it instead.
        // Re-anchoring puts it directly above the game again, which is below anything above the
        // game but still above the game itself.
        platform::anchor_overlay(window);
    }

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
/// A thread rather than a Tauri interval because the work is a plain window-system round trip
/// with no interaction with the app's state, and because the thing it replaces — a keep-above or
/// topmost claim — cannot express "above this one window only" at all (see
/// `platform::anchor_overlay`).
///
/// **This is what makes "always above the game, still below anything the player raises over it"
/// true.** A window in the keep-above or topmost state floats over every other window, including
/// ones the player deliberately brought forward; restacking expresses the narrower relationship
/// that was actually asked for.
///
/// When another client holds an **active** fullscreen window the watcher switches the overlay to
/// override-redirect, which is the one unprivileged way above it (ADR 0015). KWin's
/// `Window::belongsToLayer` returns `OverlayLayer` for an unmanaged window *before* it consults
/// `isActiveFullScreen()`, whereas a `keepAbove` toplevel can only reach `AboveLayer`, which is
/// below the fullscreen client's `ActiveLayer`. The switch is tied to the game being the *active*
/// window rather than merely fullscreen, because an unmanaged window cannot go behind anything and
/// `other_fullscreen` stays true after the player raises something over the game — claiming then
/// would cover the window they just raised. It is released as soon as the game stops being active,
/// on a switch to analysis mode, and when the overlay is hidden.
///
/// Windows needs none of that — see `crate::platform::windows` — but it runs the same loop, which
/// is the point of putting the platform difference behind [`platform::AnchorOutcome`].
///
/// @param window The overlay window.
fn watch_overlay_stacking(window: WebviewWindow) {
    use std::sync::atomic::Ordering;

    // A process-wide flag rather than a per-call one: only one watcher may exist, and the
    // atomic swap here is what enforces that. Two watchers would fight over the same stacking
    // slot several times a second.
    if OVERLAY_WATCH_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || {
        // Only changed on a transition: the flag is a window property, and setting it every
        // 400 ms would send a request per tick for a value that is almost always unchanged.
        // Starts at `false` because this is the first code to touch the flag — `apply_window_mode`
        // deliberately leaves it alone so that this belief and the window's real state cannot
        // disagree. Starting at `true` here while nothing had set the flag meant the first poll
        // computed "already correct" and never claimed the top at all.
        let mut keep_above = false;
        // The last game rect the overlay was placed against, so a poll that finds the game in the
        // same place does not issue a move request. Comparing here rather than inside
        // `set_position` keeps the decision where the previous state lives.
        let mut last_anchor: Option<(i32, i32)> = None;
        while OVERLAY_WATCH_RUNNING.load(Ordering::Relaxed) {
            // Debug hook kept out of the shipped path: a real match cannot be scripted, so the only
            // way to observe what the app concluded about the compositor is to ask it to say so.
            let debug = std::env::var_os("SC2SV_DEBUG_STACK").is_some();
            let _guard = OVERLAY_STACK_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            // Re-read visibility once the lock is held: the hotkey may have hidden the window
            // between the check above and here, and acting on a stale "visible" is what made the
            // flip re-show a window the player had just dismissed.
            if window.is_visible().unwrap_or(false) {
                let outcome = platform::anchor_overlay(&window);
                match outcome {
                    platform::AnchorOutcome::Anchored(placement) => {
                        if debug {
                            // One line per poll, because a real match cannot be scripted and the
                            // compositor's opinion of the window cannot be read back from inside
                            // the app. It is the only way to tell "the watcher decided wrong"
                            // apart from "the watcher never ran".
                            eprintln!(
                                "stack: other_fullscreen={} game_active={}",
                                placement.other_fullscreen, placement.game_active
                            );
                        }
                        keep_above = apply_overlay_stacking(
                            &window,
                            placement.other_fullscreen,
                            placement.game_active,
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
                    // unrelated windows while the player is doing something else. The watcher
                    // stays alive because the game is expected to come back.
                    platform::AnchorOutcome::NoGame => {
                        keep_above =
                            apply_overlay_stacking(&window, false, false, false, keep_above);
                        last_anchor = None;
                    }
                    // This platform cannot track another application's window at all. Stop rather
                    // than re-asking several times a second for a capability that will never
                    // appear. Reached on a Wayland-only Linux session (no X ids) and on macOS.
                    platform::AnchorOutcome::Unsupported => {
                        eprintln!(
                            "the overlay cannot be anchored to the game on this display server; \
					 it will still open, but it may not stay above the game. See docs/adr/0008"
                        );
                        break;
                    }
                }
            }
            std::thread::sleep(platform::STACKING_POLL);
        }

        // Cleared on every exit path, including the `break` above, so a later mode change can
        // start a fresh watcher rather than finding the flag stuck on.
        OVERLAY_WATCH_RUNNING.store(false, Ordering::SeqCst);
    });
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

/// Whether this particular launch should start with no visible window.
///
/// The stored setting answers "what should happen when I double-click the app", but the same
/// executable is also started by the watcher for reasons the player did not just express:
///
/// - **SC2 started** → the GUI must be silent, because a match has not begun and the player is
///   still in the menus. A window appearing here would be the app interrupting them.
/// - **the player clicked the app icon** → the window must be visible, whatever the setting says,
///   because clicking an app and getting nothing is not a reasonable outcome.
///
/// The watcher knows which of those it is, so it passes `SC2SV_START_HIDDEN` and this function
/// prefers it over the stored setting. Only the watcher-visible polarity is overridden: an
/// absent variable means the app was started some other way (a shell, `tauri dev`, a debugger)
/// and the setting should apply as normal.
///
/// @param app The running app handle.
/// @returns `true` when this launch should not show a window yet.
fn start_hidden_for_this_launch(app: &tauri::AppHandle) -> bool {
    match std::env::var("SC2SV_START_HIDDEN").ok().as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => Settings::load(app).start_hidden,
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
    if start_hidden_for_this_launch(&app) {
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
    if start_hidden_for_this_launch(&app) {
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
    let mut settings = Settings::load(&app);
    // Reported from the filesystem rather than from the stored value, because the login entry is
    // the thing that decides whether the app starts at login, and the user can remove it from
    // their desktop's own startup settings without going through this app. Reading it back means
    // the switch always shows what will actually happen at the next login.
    settings.launch_at_login = autostart::is_enabled();
    settings
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

/// Returns the stored appearance theme.
///
/// Read separately from [`get_settings`] because the frontend needs it *before* the first paint,
/// while the settings page needs the whole document much later. Fetching the whole document to
/// learn one string would put a larger IPC round trip on the critical path of every launch.
///
/// @param app The running app handle.
/// @returns The stored theme id, or the default when the stored one is not a known theme.
#[tauri::command]
fn get_theme(app: tauri::AppHandle) -> String {
    let settings = Settings::load(&app);
    if settings::THEMES.contains(&settings.theme.as_str()) {
        settings.theme
    } else {
        settings::DEFAULT_THEME.to_string()
    }
}

/// Stores the appearance theme.
///
/// Rejects an unknown theme instead of persisting it: the frontend is the only caller, so an
/// unrecognised id means a version skew between the bundle and the shell, and writing it would
/// leave the app on unstyled fallbacks with nothing to explain why. Returning the error lets the
/// settings page say so rather than silently showing a theme that is not in effect.
///
/// @param app The running app handle.
/// @param theme The requested theme id.
/// @returns The stored theme id.
#[tauri::command]
fn set_theme(app: tauri::AppHandle, theme: String) -> Result<String, String> {
    if !settings::THEMES.contains(&theme.as_str()) {
        return Err(format!(
            "{theme} is not a theme this build knows; expected one of {}",
            settings::THEMES.join(", ")
        ));
    }
    let mut current = Settings::load(&app);
    current.theme = theme.clone();
    current.save(&app)?;
    Ok(theme)
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
            // Hiding a window that is override-redirect leaves it unmanaged when it comes
            // back, and this setting is exactly "never show it unasked". Restoring management
            // here means the next reveal behaves like an ordinary window.
            release_unmanaged(&window);
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
/// The login entry and the stored setting are changed together, in that order: if writing the
/// entry fails the setting is not persisted, or the settings page and the user's login
/// behaviour would disagree. The entry names the **watcher**, not this program, which is why it
/// is written here rather than by a Tauri plugin — see [`autostart`].
///
/// @param app The running app handle.
/// @param enabled Whether to launch the app at login.
/// @returns The stored value.
#[tauri::command]
fn set_launch_at_login(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    if !Settings::autostart_available(&app) {
        return Err(
            "launch at login needs an installed build; run the app from a bundle or package \
             rather than from a development binary"
                .to_string(),
        );
    }

    autostart::set_enabled(enabled)?;

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
            "quit" => {
                // The watcher supervises this process, so quitting only the window would leave
                // it to notice the exit and start a fresh one the next time SC2 appears —
                // the app would seem to ignore "Quit". Asking the watcher to stop retires it
                // too; when the GUI was started by hand there is no watcher listening and the
                // send simply fails, which is the correct outcome.
                control::send_to(control::WATCHER_CONTROL_NAME, control::QUIT);
                app.exit(0);
            }
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
        // Reveals the main window once the frontend has painted (see `reveal_main_window`).
        .invoke_handler(tauri::generate_handler![
            frontend_ready,
            toggle_window,
            get_window_mode,
            set_window_mode_command,
            reveal_window_mode_command,
            overlay_supported_command,
            animations_supported_command,
            get_settings,
            get_theme,
            set_theme,
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

            // Answer show requests from the watcher, so a second launch or the game starting
            // can reveal a GUI that was spawned hidden. See `gui_control`.
            start_gui_listener(app.handle().clone());

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
                    // Keep the affordance from being undone by the frontend a second later: the
                    // frontend decides the mode from the client API on every poll, and a value
                    // left set means "stay in overlay mode" no matter what the polling says.
                    // Without this the only way to inspect the overlay's real compositor
                    // placement is to be in a live match, which cannot be scripted.
                    if std::env::var_os("SC2SV_PIN_OVERLAY")
                        .is_some_and(|value| value != "0")
                    {
                        PINNED_OVERLAY.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
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

/// Serialises every change to the window's managed state.
///
/// Flipping override-redirect requires hiding and re-showing the window (a `GdkWindow` cannot
/// change the attribute while it exists). That hide/show pair is two separate window
/// operations, so if the hotkey hides the window in between them the flip's own `show()` wins
/// and the overlay comes back — measured: three consecutive keypresses all reported
/// `is_visible=true`, because the watcher pokes the window every 400 ms and one of those pokes
/// landed inside the hide. Taking this lock in both the watcher tick and the hotkey path makes
/// the two orderings the only two that can happen.
static OVERLAY_STACK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Whether the stacking watcher thread is alive.
///
/// Declared on every platform rather than Linux-only: the watcher runs everywhere now, and it is
/// the watcher's own `AnchorOutcome::Unsupported` report that ends it on a platform that cannot
/// track another window, not a `cfg`.
static OVERLAY_WATCH_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the window is currently override-redirect (unmanaged), i.e. above a fullscreen game.
///
/// Process-wide because it describes one window, and the two things that change it — the mode
/// switch and the stacking watcher — are on different threads. Reading and writing it through an
/// atomic is what stops them from each flipping it and leaving the window in whichever state won
/// the race.
static WINDOW_UNMANAGED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the development affordance has locked the window into overlay mode.
///
/// Only ever set from `SC2SV_START_WITH_OVERLAY` + `SC2SV_PIN_OVERLAY`, which exist so the
/// overlay's real compositor placement can be inspected outside of a live match. It is read by
/// [`set_window_mode_command`], which otherwise lets the frontend's automatic decision win.
static PINNED_OVERLAY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Makes the window an ordinary managed window again, if it is not one already.
///
/// Every transition *out* of the fullscreen-overlay state calls this. It is deliberately
/// idempotent and cheap when there is nothing to do, because it is called from paths that run
/// on every poll and from paths that run once per mode change.
///
/// Returns whether it actually released anything, because the flip re-shows the window (GDK has
/// to recreate the X resource), and showing a window raises it to the top of the stack. The caller
/// has to repair the stacking afterwards or the overlay would end up above a window the player has
/// just raised over the game — measured exactly that way round: `RAISED` at stack position 4 and
/// the overlay at 5, when the overlay was supposed to have gone behind it.
///
/// @param window The window to restore.
/// @returns `true` when the window was override-redirect and is now managed.
fn release_unmanaged(window: &WebviewWindow) -> bool {
    release_unmanaged_and_visibility(window, true)
}

/// [`release_unmanaged`], saying whether the window must be left on screen afterwards.
///
/// A hide-then-release (the hotkey path) must pass `false`: the release flips the attribute by
/// hiding and re-showing the window, so leaving the restore on would re-show a window the player
/// just hid. See [`platform::set_unmanaged_and_visibility`].
///
/// @param window The window to restore.
/// @param restore_visibility Whether to leave the window visible, as it was found.
/// @returns `true` when the window was override-redirect and is now managed.
fn release_unmanaged_and_visibility(window: &WebviewWindow, restore_visibility: bool) -> bool {
    use std::sync::atomic::Ordering;
    if WINDOW_UNMANAGED.swap(false, Ordering::SeqCst) {
        platform::set_unmanaged_and_visibility(window, false, restore_visibility);
        // The release is the same hide/show cycle as the claim, so it has the same fault: the
        // fresh managed window is placed by the window manager rather than at the anchor it had
        // before. Re-asserting it here keeps the overlay in the game's corner across a
        // flip back to managed, instead of jumping to the middle of the screen.
        if restore_visibility {
            reposition_for_overlay(window, None);
        }
        // Defensive: an earlier build claimed always-on-top alongside the override-redirect
        // state, and a window carrying that flag after being released is in `AboveLayer`, which
        // is above any ordinary window the player raises. Clearing it here means the release
        // path cannot inherit that stale claim on any code path.
        let _ = window.set_always_on_top(false);
        true
    } else {
        false
    }
}

/// Makes the window override-redirect so it can sit above an active fullscreen client.
///
/// Returns whether the window is now unmanaged, so the caller does not have to guess: on a
/// display server that cannot do this the answer is `false` and the caller keeps its ordinary
/// keep-above behaviour instead of believing it is on top when it is not.
///
/// @param window The window to raise above a fullscreen client.
/// @returns `true` when the window is now override-redirect.
fn claim_unmanaged(window: &WebviewWindow) -> bool {
    use std::sync::atomic::Ordering;
    if WINDOW_UNMANAGED.load(Ordering::SeqCst) {
        return true;
    }
    if platform::set_unmanaged(window, true) {
        WINDOW_UNMANAGED.store(true, Ordering::SeqCst);
        // The claim hides and re-shows the window, which destroys the `GdkWindow` and lets the
        // toolkit and the window manager place the new one from scratch. Nothing in the flip
        // carries the old position over, and the window manager's default for a fresh unmanaged
        // window is the centre of the screen — measured: an overlay anchored at `(24, 24)`
        // reappeared at `(1730, 754)`, exactly the centre of a 3840x2160 screen for a 380x600
        // window. Because the re-show lands *after* the caller's move request, the visible result
        // is the reported "briefly moves to the correct position, then snaps back to the centre".
        //
        // Re-asserting the anchor here is what makes the position survive the flip. The game
        // rectangle is looked up again rather than threaded through the callers, so this is
        // correct whichever path reached it, and it costs one lookup per game rather than per
        // poll.
        reposition_for_overlay(window, None);
        // Click-through is a window-manager property that a hide/show cycle drops, and this
        // window is being re-shown precisely while the player is mid-game. Losing it here would
        // put an invisible 380x300 rectangle over the game that eats every click aimed at
        // whatever is underneath it.
        let _ = window.set_ignore_cursor_events(true);
        // Deliberately NOT `set_always_on_top(true)`. An override-redirect window needs no claim:
        // KWin puts an unmanaged window above every managed layer regardless, and the claim
        // actively breaks the release path. The claim is a layer, so `AboveLayer` keeps the
        // overlay above a window the player raised over the game (`NormalLayer`) no matter how
        // the X stack is ordered — measured: the overlay sat at `layer=3 keepAbove=true` above
        // `RAISED` at `layer=2`, which is the opposite of "go behind other windows". Worse, it is
        // invisible to the watcher's own bookkeeping, which tracks the flag and therefore never
        // saw a reason to clear it.
        eprintln!(
            "the game is running fullscreen, so the overlay went above it as an unmanaged \
             window (see docs/adr/0015-raise-the-overlay-with-override-redirect.md)"
        );
        true
    } else {
        false
    }
}

/// The colour the analysis view's web view is given, so it never composites onto nothing.
///
/// Must stay equal to the frontend's `--color-surface-950` (`oklch(0.16 0.012 258)`, which is
/// `#0a0d12`). The page paints the same colour on its own surface, so the two agree and the
/// window has no visible seam while the page is still loading. That agreement is not what makes
/// the window opaque — the web view's own background is — but a mismatch would show as a flash of
/// the wrong shade on every reveal.
const ANALYSIS_BACKGROUND: tauri::webview::Color =
    tauri::webview::Color(0x0a, 0x0d, 0x12, 0xff);

/// The zero-alpha colour that makes the overlay's web view see-through.
///
/// A named constant rather than `None` at the call site, because `None` means *opaque white* in
/// this stack (see the comment where it is used) — the opposite of what the overlay needs.
const TRANSPARENT_BACKGROUND: tauri::webview::Color = tauri::webview::Color(0, 0, 0, 0);

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
///
/// Twice the first attempt's 300 on the player's request: the guide's whole job is showing the
/// upcoming steps, and at 300 logical pixels only about four were visible at a time, so it was
/// being read one step ahead. Height is the constraint that matters, not width.
const OVERLAY_HEIGHT: f64 = 600.0;

#[cfg(test)]
mod requested_mode_tests {
    use super::{requested_mode, WindowMode};

    /// The two names the frontend actually sends, and the fallback for anything else. An
    /// unrecognised payload resolves to analysis because that is the mode the player can still
    /// see and click — a typo must not leave them holding a click-through panel.
    #[test]
    fn a_mode_name_maps_to_that_mode_and_junk_maps_to_analysis() {
        assert_eq!(requested_mode("overlay"), WindowMode::Overlay);
        assert_eq!(requested_mode("analysis"), WindowMode::Analysis);
        assert_eq!(requested_mode(""), WindowMode::Analysis);
        assert_eq!(requested_mode("Overlay"), WindowMode::Analysis);
    }
}

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

#[cfg(test)]
mod hotkey_hide_tests {
    use super::{hotkey_should_hide, WindowMode};

    /// The regression the user reported: in analysis mode the app is mapped but buried under an
    /// active fullscreen game, so a press must reveal rather than hide. Before this rule the
    /// press hid the buried window, which looked exactly like a key that does nothing, and then
    /// appeared to work on the next press.
    #[test]
    fn a_mapped_analysis_window_hidden_behind_the_game_is_revealed_not_hidden() {
        assert!(!hotkey_should_hide(WindowMode::Analysis, true, false));
    }

    /// The ordinary analysis case the toggle is for: the app is on screen and in front, so the
    /// press dismisses it.
    #[test]
    fn a_mapped_active_analysis_window_is_hidden() {
        assert!(hotkey_should_hide(WindowMode::Analysis, true, true));
    }

    /// The regression that made the key reveal-only inside a match. An overlay is click-through
    /// and refuses focus, so it is never the active window; requiring `active` therefore meant
    /// every press revealed. Measured before this: `is_visible=true is_active=false
    /// hiding=false mode=Overlay`, i.e. the guide flickered and came back instead of closing.
    #[test]
    fn a_mapped_overlay_is_hidden_even_though_it_can_never_be_active() {
        assert!(hotkey_should_hide(WindowMode::Overlay, true, false));
        assert!(hotkey_should_hide(WindowMode::Overlay, true, true));
    }

    /// A window that is not mapped at all is revealed, which is the plain "show me the app"
    /// press. Neither the active flag nor the mode may flip that decision.
    #[test]
    fn a_hidden_window_is_revealed_whatever_the_active_flag_says() {
        for mode in [WindowMode::Analysis, WindowMode::Overlay] {
            assert!(!hotkey_should_hide(mode, false, false));
            assert!(!hotkey_should_hide(mode, false, true));
        }
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

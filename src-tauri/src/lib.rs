//! The Rust shell for sc2-science-vessel.
//!
//! Its job is deliberately thin. It owns the main window, the overlay window, the single
//! global shortcut, and the lifecycle of the Python analysis sidecar (ADR 0001). It does
//! **not** parse replays, compute metrics, or decide anything about build orders — those
//! live in the sidecar and the frontend.
//!
//! Two constraints from the feasibility study shape this file and must not be broken:
//!
//! - The app never reads the game process's memory, and never sends input to the game.
//!   The only sanctioned live input is the SC2 Client API on `localhost:6119`, which the
//!   frontend polls directly (ADR 0004). This is what keeps the overlay on the benign
//!   side of Blizzard's discretionary EULA clause.
//! - The overlay is a plain always-on-top, click-through window for now. KDE exposes
//!   `zwlr_layer_shell_v1`, so a real compositor-level overlay is reachable and is the
//!   planned upgrade (ADR 0003); it will be a native window beside this one, not a
//!   WebView trick.

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Label of the always-on-top build-order overlay window, as declared in `tauri.conf.json`.
pub const OVERLAY_WINDOW_LABEL: &str = "overlay";

/// Hides the overlay if it is visible, shows it if it is hidden, and returns the new state.
///
/// Toggling a window is not a single Tauri call: a window can be *closed* rather than
/// hidden, so this recreates it on demand. It also re-applies the click-through and
/// always-on-top flags on every show, because those flags are lost when a window is
/// recreated and on some platforms when it is re-shown.
///
/// @param app The running app handle.
/// @returns `true` if the overlay is now visible.
pub fn toggle_overlay(app: &tauri::AppHandle) -> tauri::Result<bool> {
	if let Some(window) = app.get_webview_window(OVERLAY_WINDOW_LABEL) {
		if window.is_visible().unwrap_or(false) {
			window.hide()?;
			return Ok(false);
		}
		apply_overlay_flags(&window)?;
		window.show()?;
		window.set_focus()?;
		return Ok(true);
	}

	let window = create_overlay_window(app)?;
	apply_overlay_flags(&window)?;
	window.show()?;
	Ok(true)
}

/// Builds and runs the application.
///
/// Registers the plugins the shell needs and wires the global shortcut that toggles the
/// overlay. The shortcut defaults to `Ctrl+Alt+B` because it is unlikely to collide with
/// a StarCraft II hotkey; it is stored here rather than in config so the eventual settings
/// UI has one obvious place to change it.
pub fn run() {
	tauri::Builder::default()
		.plugin(tauri_plugin_http::init())
		.plugin(tauri_plugin_shell::init())
		// The shortcut is registered from inside `setup` rather than by the builder so a
		// failure is recoverable. On Wayland registration is expected to fail — the plugin
		// is X11-only — and an error returned from `setup` would take the whole app down
		// with it, leaving the player with no window to toggle the overlay from either.
		.setup(|app| {
			use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
			let toggle = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyB);
			if let Err(err) = app.global_shortcut().register(toggle) {
				eprintln!(
					"global shortcut unavailable ({err}); \
					 the overlay can still be toggled from the app window. \
					 Wayland needs the XDG GlobalShortcuts portal — see docs/adr/0003"
				);
			}
			Ok(())
		})
		.plugin(
			tauri_plugin_global_shortcut::Builder::new()
				.with_handler(|app, _shortcut, event| {
					// Act on press only: a handler that also fired on release would toggle
					// the overlay twice per keystroke and net out to no visible change.
					if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
						let _ = toggle_overlay(app);
					}
				})
				.build(),
		)
		.run(tauri::generate_context!())
		.expect("error while running sc2-science-vessel");
}

/// Builds the overlay window, pointing it at the `/overlay` route of the frontend.
///
/// The overlay is an ordinary borderless always-on-top WebView with click-through
/// enabled. That works on Windows, X11, and macOS over a window, and on KDE Wayland it
/// works when the game runs windowed or borderless-fullscreen. It does not reliably cover
/// a truly fullscreen game on Wayland, because KWin draws an overlay surface above
/// everything *except* fullscreen windows; the fix is a native `wlr-layer-shell` surface
/// in the `overlay` layer, which this machine's compositor advertises (ADR 0003).
///
/// The frontend route is SPA-routed, so the target is the app's own `overlay` path; with
/// `adapter-static`'s `index.html` fallback the WebView loads the shell and the client
/// router resolves `/overlay` from the URL.
///
/// @param app The running app handle.
/// @returns The newly created overlay window.
fn create_overlay_window(app: &tauri::AppHandle) -> tauri::Result<WebviewWindow> {
	WebviewWindowBuilder::new(app, OVERLAY_WINDOW_LABEL, WebviewUrl::App("overlay".into()))
		.title("Build Order")
		.inner_size(380.0, 300.0)
		.position(24.0, 96.0)
		.resizable(true)
		.decorations(false)
		.always_on_top(true)
		.skip_taskbar(true)
		.shadow(false)
		.visible(false)
		.build()
}

/// Applies the window flags that make the overlay usable while playing.
///
/// These are re-applied every time the window is shown, not only at creation, because
/// `always_on_top` and cursor-event passthrough are not guaranteed to survive a hide/show
/// cycle — and a click-through overlay that has quietly become clickable is a bug the
/// player would only notice mid-game.
///
/// Click-through is unconditional on purpose: the overlay exists to sit on screen while
/// the player uses the game underneath, so there is no state in which intercepting the
/// mouse would be correct.
///
/// @param window The overlay window to configure.
fn apply_overlay_flags(window: &WebviewWindow) -> tauri::Result<()> {
	window.set_always_on_top(true)?;
	window.set_ignore_cursor_events(true)?;
	Ok(())
}

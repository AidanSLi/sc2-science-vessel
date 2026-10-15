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
use std::path::Path;

mod settings;
mod sidecar;

pub use settings::Settings;

/// Label of the always-on-top build-order overlay window, as declared in `tauri.conf.json`.
pub const OVERLAY_WINDOW_LABEL: &str = "overlay";

/// Command the frontend calls to show or hide the overlay window.
///
/// Hides the overlay if it is visible, shows it if it is hidden, and returns the new state.
///
/// Toggling a window is not a single Tauri call: a window can be *closed* rather than
/// hidden, so this recreates it on demand. It also re-applies the click-through and
/// always-on-top flags on every show, because those flags are lost when a window is
/// recreated and on some platforms when it is re-shown.
///
/// Exposed as a command so the frontend has exactly **one** way to bring the overlay up. It
/// must not build a `WebviewWindow` itself: the flags that make the overlay usable are
/// applied here, and a window created from the frontend would look right and then behave
/// like an ordinary window (see `apply_overlay_flags`).
///
/// @param app The running app handle.
/// @returns `true` if the overlay is now visible.
#[tauri::command]
fn toggle_overlay(app: tauri::AppHandle) -> tauri::Result<bool> {
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

	let window = create_overlay_window(&app)?;
	apply_overlay_flags(&window)?;
	window.show()?;
	Ok(true)
}

/// Label of the main window declared in `tauri.conf.json`.
pub const MAIN_WINDOW_LABEL: &str = "main";

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
/// @param app The running app handle.
fn arm_main_window_watchdog(app: tauri::AppHandle) {
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
#[tauri::command]
fn frontend_ready(app: tauri::AppHandle) {
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
		// Reveals the main window once the frontend has painted (see `reveal_main_window`).
		.invoke_handler(tauri::generate_handler![
			frontend_ready,
			toggle_overlay,
			get_settings,
			set_overlay_shortcut,
			suggest_replay_roots,
			set_replay_roots,
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
			// Register from the stored setting, not a constant, so a rebind survives a
			// restart. `register_overlay_shortcut` walks a fallback list when the stored key
			// is taken, so a fresh install still gets a working toggle key.
			let loaded = Settings::load(app.handle());
			match settings::register_overlay_shortcut(app.handle(), &loaded.overlay_shortcut) {
				Ok(registered) => {
					if registered != loaded.overlay_shortcut {
						eprintln!(
							"overlay shortcut {} was unavailable; registered {registered} instead",
							loaded.overlay_shortcut
						);
					}
					// Persist the substitution so the settings page shows the key that actually
					// works rather than the one that failed to register.
					if registered != loaded.overlay_shortcut {
						let mut updated = loaded.clone();
						updated.overlay_shortcut = registered;
						if let Err(err) = updated.save(app.handle()) {
							eprintln!("could not persist the fallback shortcut ({err})");
						}
					}
				}
				Err(err) => eprintln!(
					"no global shortcut could be registered ({err}); \
					 the overlay can still be toggled from the app window. \
					 Wayland needs the XDG GlobalShortcuts portal — see docs/adr/0003"
				),
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
						let _ = toggle_overlay(app.clone());
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

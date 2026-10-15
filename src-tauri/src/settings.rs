//! User settings, persisted next to the app's other data.
//!
//! Deliberately a small JSON file rather than a config crate: there is exactly one
//! user-changeable setting so far (the overlay toggle key), and a JSON blob that fails to
//! parse falls back to defaults instead of refusing to start. The file is written on change,
//! not on exit, so a crash mid-game cannot lose a setting the player just changed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// The overlay toggle key used when the user has never chosen one.
///
/// Not `Ctrl+Alt+B`: that combination is taken on at least one real KDE session, and a
/// default that silently fails to register would make the overlay look unreachable on a
/// fresh install. `Ctrl+Alt+O` is unclaimed in a stock KDE keymap (which reserves only
/// `Ctrl+Alt+Del`) and does not collide with a StarCraft II default hotkey.
pub const DEFAULT_OVERLAY_SHORTCUT: &str = "Ctrl+Alt+O";

/// Keys tried, in order, when the configured key cannot be registered.
///
/// Registration fails when another application already owns the combination, and the
/// global-shortcut plugin cannot tell us that in advance — `is_registered` only reports the
/// plugin's own registrations, not the compositor's. So the only way to discover a conflict
/// is to try, and the only useful response is to try the next candidate. Falling through
/// this list means the overlay still has *a* working key rather than none, which matters
/// because the key is the whole point of the overlay: it is pressed mid-game, with the game
/// holding keyboard focus.
pub const FALLBACK_OVERLAY_SHORTCUTS: &[&str] = &[
	"Ctrl+Alt+O",
	"Ctrl+Alt+U",
	"Ctrl+Alt+F9",
	"Ctrl+Shift+F9",
	"Ctrl+Alt+Y",
];

/// The persisted settings document.
///
/// Every field carries `#[serde(default)]` so that a file written by an older build (or a
/// hand-edited one) still loads: a missing field becomes its default rather than a parse
/// error that would discard the whole document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
	/// Key combination that shows and hides the overlay, in accelerator form
	/// (`Ctrl+Alt+O`). Parsed by the global-shortcut plugin, which accepts `Ctrl`, `Alt`,
	/// `Shift`, `Super`/`Cmd` modifiers and a trailing key.
	#[serde(default = "default_overlay_shortcut")]
	pub overlay_shortcut: String,
}

/// Returns the default accelerator as an owned string, for serde's `default` attribute.
fn default_overlay_shortcut() -> String {
	DEFAULT_OVERLAY_SHORTCUT.to_string()
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			overlay_shortcut: default_overlay_shortcut(),
		}
	}
}

impl Settings {
	/// Loads settings from disk, falling back to defaults on any failure.
	///
	/// A missing file is the normal first-run case. A *corrupt* file is reported on stderr
	/// and then ignored: refusing to start because a settings blob could not be parsed would
	/// trade a minor annoyance for a completely unusable app.
	///
	/// @param app The running app handle, used to locate the config directory.
	/// @returns The loaded settings, or defaults.
	pub fn load(app: &AppHandle) -> Self {
		let path = match settings_path(app) {
			Ok(path) => path,
			Err(err) => {
				eprintln!("settings path unavailable ({err}); using defaults");
				return Self::default();
			}
		};
		match std::fs::read_to_string(&path) {
			Ok(text) => match serde_json::from_str::<Self>(&text) {
				Ok(settings) => settings,
				Err(err) => {
					eprintln!("{} is unreadable ({err}); using defaults", path.display());
					Self::default()
				}
			},
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
			Err(err) => {
				eprintln!("{} could not be read ({err}); using defaults", path.display());
				Self::default()
			}
		}
	}

	/// Writes settings to disk, creating the config directory if needed.
	///
	/// @param app The running app handle, used to locate the config directory.
	/// @returns `Err` with a human-readable reason if the write failed.
	pub fn save(&self, app: &AppHandle) -> Result<(), String> {
		let path = settings_path(app)?;
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
		}
		let text = serde_json::to_string_pretty(self)
			.map_err(|err| format!("settings could not be serialised: {err}"))?;
		std::fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))
	}
}

/// Resolves the settings file path inside the app's config directory.
///
/// Uses Tauri's own config directory rather than a hand-built path so it lands where the
/// platform expects (`~/.config/<identifier>` on Linux, `%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS).
///
/// @param app The running app handle.
/// @returns The full path to `settings.json`.
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
	let dir = app
		.path()
		.app_config_dir()
		.map_err(|err| format!("no config directory: {err}"))?;
	Ok(dir.join("settings.json"))
}

/// Registers the overlay shortcut, falling back through [`FALLBACK_OVERLAY_SHORTCUTS`].
///
/// Clears any previously registered shortcut first, so this doubles as the rebind path when
/// the user picks a new key from the settings page. Returns the accelerator that ended up
/// registered, which may differ from the requested one — the UI must show what it *got*, not
/// what it asked for, or a user whose key was taken would keep pressing a dead combination.
///
/// @param app The running app handle.
/// @param requested The accelerator the user configured.
/// @returns The accelerator actually registered, and an error string if none could be.
pub fn register_overlay_shortcut(app: &AppHandle, requested: &str) -> Result<String, String> {
	use tauri_plugin_global_shortcut::GlobalShortcutExt;

	let manager = app.global_shortcut();
	// Unregistering everything is safe here: the overlay toggle is the app's only shortcut,
	// and a stale registration would make the next `register` fail as a duplicate.
	let _ = manager.unregister_all();

	let mut last_error = String::new();
	let mut candidates: Vec<&str> = vec![requested];
	for candidate in FALLBACK_OVERLAY_SHORTCUTS {
		if !candidates.contains(candidate) {
			candidates.push(candidate);
		}
	}

	for candidate in candidates {
		match manager.register(candidate) {
			Ok(()) => return Ok(candidate.to_string()),
			Err(err) => {
				last_error = format!("{candidate}: {err}");
			}
		}
	}
	Err(last_error)
}

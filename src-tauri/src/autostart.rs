//! The "launch at login" setting, which has to start the **watcher** rather than the window.
//!
//! ## Why this is not `tauri-plugin-autostart`
//!
//! That plugin was used here until the app split into two binaries. It writes an entry whose
//! `Exec` is `std::env::current_exe()`, which is necessarily *the process that asked for it* —
//! and the process that asks is the GUI, since that is where the settings page lives. A login
//! entry pointing at the GUI starts a window with no watcher behind it, so the player would have
//! a window at login and no game detection at all. The plugin has no way to name a different
//! program, and that is not a gap worth working around inside a plugin: the file it writes on
//! Linux is twelve lines and is specified by the freedesktop autostart spec.
//!
//! So this module writes the entry itself, naming the watcher sibling of the running executable.
//!
//! ## The `.desktop` file
//!
//! Written to `$XDG_CONFIG_HOME/autostart/<name>.desktop`, which is where every desktop
//! environment looks for login entries. `--watched-by-desktop` is passed so the watcher can tell
//! a login launch (stay silent, wait for the game) from the player clicking the app icon (show a
//! window now); the same binary serves both, and nothing else distinguishes them.
//!
//! Only Linux is implemented. Windows and macOS have their own startup mechanisms, and this
//! module returns an explicit error there rather than pretending: silently doing nothing would
//! leave a settings toggle that appears to work and never does.

use std::path::PathBuf;

/// The file name the login entry is written under.
///
/// Must match what `uninstall` removes and what an installed `.desktop` file is called, so that
/// the desktop can associate the login entry with the application it belongs to.
pub const ENTRY_NAME: &str = "sc2-science-vessel";

/// The name of the watcher binary, which is what a login entry must start.
pub const WATCHER_BINARY_NAME: &str = "sc2sv-watch";

/// The flag that tells the watcher this launch came from the desktop session.
pub const AUTOSTART_FLAG: &str = "--watched-by-desktop";

/// Where the login entry is written.
///
/// @returns The path of the `.desktop` file, under `$XDG_CONFIG_HOME/autostart` when that is
///   set and `~/.config/autostart` otherwise, which is the order the spec specifies.
pub fn entry_path() -> Option<PathBuf> {
	let base = std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
	Some(base.join("autostart").join(format!("{ENTRY_NAME}.desktop")))
}

/// The watcher binary that a login entry should start.
///
/// A sibling of the running executable, which is how both `tools/install_dev.sh` and the packaged
/// bundles lay the two files out.
///
/// @returns The watcher's path, or `None` when this executable's own location is unknown.
pub fn watcher_path() -> Option<PathBuf> {
	let own = std::env::current_exe().ok()?;
	let sibling = own.parent()?.join(if cfg!(windows) {
		format!("{WATCHER_BINARY_NAME}.exe")
	} else {
		WATCHER_BINARY_NAME.to_string()
	});
	Some(sibling)
}

/// Whether writing a login entry for this build makes sense.
///
/// Two conditions, both of which a development build fails: the watcher must exist next to this
/// executable (it does not under `cargo run`, which produces only the GUI), and the executable
/// must not be inside a `target/` directory, where the next build replaces or deletes it and the
/// login entry breaks.
///
/// @returns `true` when enabling launch-at-login would produce a working entry.
pub fn available() -> bool {
	#[cfg(target_os = "linux")]
	{
		let Some(watcher) = watcher_path() else {
			return false;
		};
		if !watcher.is_file() || entry_path().is_none() {
			return false;
		}
		!std::env::current_exe().is_ok_and(|exe| {
			exe.components()
				.any(|part| part.as_os_str() == "target")
		})
	}
	#[cfg(not(target_os = "linux"))]
	{
		false
	}
}

/// Whether a login entry currently exists.
///
/// @returns `true` when the entry file is present.
pub fn is_enabled() -> bool {
	entry_path().is_some_and(|path| path.exists())
}

/// Writes or removes the login entry.
///
/// @param enabled Whether the app should start at login.
/// @returns `Err` with a reason when the entry could not be written or removed.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
	#[cfg(target_os = "linux")]
	{
		let path = entry_path().ok_or_else(|| {
			"neither XDG_CONFIG_HOME nor HOME is set, so there is nowhere to write a login entry"
				.to_string()
		})?;

		if !enabled {
			return match std::fs::remove_file(&path) {
				Ok(()) => Ok(()),
				Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
				Err(err) => Err(format!("{}: {err}", path.display())),
			};
		}

		let watcher = watcher_path()
			.ok_or_else(|| "could not determine where the app is installed".to_string())?;
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
		}

		std::fs::write(&path, entry_contents(&watcher.display().to_string()))
			.map_err(|err| format!("{}: {err}", path.display()))
	}
	#[cfg(not(target_os = "linux"))]
	{
		let _ = enabled;
		Err("launch at login is only implemented on Linux so far".to_string())
	}
}

/// Renders the login entry.
///
/// Extracted so it can be asserted on without touching the filesystem. The quoting of `Exec` is
/// the part worth testing: a path containing a space would otherwise split into two arguments and
/// start the wrong program, which fails silently at login and is therefore worth preventing here
/// rather than debugging later.
///
/// @param watcher The absolute path of the watcher binary.
/// @returns The `.desktop` file's text.
pub fn entry_contents(watcher: &str) -> String {
	format!(
		"[Desktop Entry]\n\
		 Type=Application\n\
		 Name=SC2 Science Vessel\n\
		 Comment=Watches for StarCraft II and starts the build-order overlay\n\
		 Exec=\"{watcher}\" {AUTOSTART_FLAG}\n\
		 Icon={ENTRY_NAME}\n\
		 Terminal=false\n\
		 X-GNOME-Autostart-enabled=true\n"
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_entry_starts_the_watcher_with_the_autostart_flag() {
		let text = entry_contents("/opt/sc2/sc2sv-watch");
		assert!(text.contains("Exec=\"/opt/sc2/sc2sv-watch\" --watched-by-desktop"));
		assert!(text.starts_with("[Desktop Entry]\n"));
	}

	#[test]
	fn a_path_with_a_space_stays_one_argument() {
		// Without quoting this would start `/home/me/my` with an argument of `apps/...`, which
		// fails at login with no visible error.
		let text = entry_contents("/home/me/my apps/sc2sv-watch");
		assert!(text.contains("Exec=\"/home/me/my apps/sc2sv-watch\" --watched-by-desktop"));
	}

	#[test]
	fn the_entry_is_disabled_able_by_gnome() {
		assert!(entry_contents("/x/sc2sv-watch").contains("X-GNOME-Autostart-enabled=true"));
	}
}

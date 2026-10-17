//! The resident half of the app: watches for StarCraft II and owns the GUI child process.
//!
//! ## Why this is a separate binary
//!
//! The app has two roles, and for a while both were one executable chosen by `--gui`. That was
//! wrong for the resident role, and measurably so: a single ELF shares one set of dynamic
//! libraries, so the watcher mapped the entire WebKit/GTK stack it never calls into.
//!
//! | build | PSS when idle |
//! |---|---|
//! | the shared executable, both roles | 23.6 MiB |
//! | this crate, standard library only | 486 KiB |
//!
//! The user's requirement was that the resident process be "extremely small", and 23.6 MiB is
//! not small next to 486 KiB. The cost of the split is one extra file to install; the benefit is
//! a ~48x reduction in the process that runs for the whole session.
//!
//! ## Why the source is not duplicated
//!
//! The modules are pulled in by path from the parent crate, so the watcher's logic — the action
//! table, the control protocol, the game-process matcher, the instance lock — exists once and is
//! covered once. A copy would drift, and the two roles would disagree about something subtle
//! such as which process names count as the game.
//!
//! Everything here goes through `SC2SV_GUI_EXECUTABLE`, which the desktop entry and the
//! installer set. The watcher cannot find the GUI by looking beside its own executable, because
//! the two files are not necessarily co-located once a package manager has laid them out.

#[path = "../../src/control.rs"]
mod control;
#[path = "../../src/game_process.rs"]
mod game_process;
// The watcher uses only the client half of this module (asking a running window to show itself);
// the server half belongs to the GUI. The file is included whole because splitting it would put
// the two halves of one protocol in two files that must change together — a worse trade than a
// dead-code allowance on the half this process never calls.
#[allow(dead_code)]
#[path = "../../src/gui_control.rs"]
mod gui_control;
#[path = "../../src/single_instance.rs"]
mod single_instance;
#[path = "../../src/watcher.rs"]
mod watcher;

/// The name the GUI binary is installed under, beside this one.
///
/// The two binaries are installed into the same directory by both `tools/install_dev.sh` and the
/// Tauri bundles, so a sibling lookup is the whole resolution rule. An override exists only for
/// the case where they are not co-located, which a package manager is technically free to do.
pub const GUI_BINARY_NAME: &str = "sc2-science-vessel";

/// An explicit path to the GUI binary, overriding the sibling lookup.
pub const GUI_EXECUTABLE_ENV: &str = "SC2SV_GUI_EXECUTABLE";

/// Runs the resident supervisor.
///
/// A launch that finds another watcher already running asks it to show the app instead of
/// starting a second one, which is what clicking the app icon means.
fn main() {
	let Some(_lock) = single_instance::acquire() else {
		if watcher::request_show_of_running_instance() {
			eprintln!("sc2-science-vessel: already running; asked it to show the window");
			return;
		}
		eprintln!(
			"sc2-science-vessel: another instance is already running but did not respond; \
			 quit it before starting another"
		);
		std::process::exit(1);
	};

	let intent = if started_by_autostart() {
		watcher::StartIntent::Background
	} else {
		watcher::StartIntent::Interactive
	};
	std::process::exit(watcher::run(intent, spawn_gui));
}

/// Whether this launch is the desktop session's autostart entry.
///
/// The autostart entry passes `--watched-by-desktop`, which is the difference between "start
/// silently and wait for the game" and "the player just clicked this, show them a window".
/// Detected from the command line rather than from a setting because it describes *this launch*,
/// not a preference: the same installed executable is both an autostart entry and a menu item.
///
/// @returns `true` when the launcher asked for a silent start.
fn started_by_autostart() -> bool {
	std::env::args().any(|argument| argument == "--watched-by-desktop")
}

/// Starts the GUI as a child process with the visibility this launch calls for.
///
/// The visibility is passed through the environment rather than the user's `start_hidden`
/// setting, because the watcher knows *why* the GUI is starting and the setting does not: a GUI
/// spawned because SC2 started must be silent, and one spawned because the player clicked the app
/// must be visible. `SC2SV_START_HIDDEN` overrides the setting for this launch only.
///
/// Nothing else is added to the child's environment on purpose: the renderer and display-backend
/// workarounds are decided in the GUI's own `main`, which is the only place that knows whether an
/// X server exists. Duplicating them here would create two sources of truth for one decision.
///
/// @param start Whether the window should appear immediately.
/// @returns The child handle, or `None` when the GUI executable is unknown or cannot be started.
fn spawn_gui(start: watcher::GuiStart) -> Option<std::process::Child> {
	let executable = gui_executable()?;
	let hidden = matches!(start, watcher::GuiStart::Hidden);

	match std::process::Command::new(&executable)
		.arg("--gui")
		.env("SC2SV_START_HIDDEN", if hidden { "1" } else { "0" })
		.spawn()
	{
		Ok(child) => {
			eprintln!(
				"sc2-science-vessel: started the app window (pid {})",
				child.id()
			);
			Some(child)
		}
		Err(err) => {
			eprintln!(
				"sc2-science-vessel: could not start {} ({err}); set {GUI_EXECUTABLE_ENV} to the \
				 app's window binary",
				executable.display()
			);
			None
		}
	}
}

/// The GUI binary to start.
///
/// The environment override is checked first, then the sibling of this executable. A sibling is
/// the normal case and needs no configuration: both binaries are installed side by side.
///
/// @returns The path to the GUI binary, or `None` when neither source yields one.
fn gui_executable() -> Option<std::path::PathBuf> {
	if let Some(configured) = std::env::var_os(GUI_EXECUTABLE_ENV) {
		if !configured.is_empty() {
			return Some(std::path::PathBuf::from(configured));
		}
	}

	let own = std::env::current_exe().ok()?;
	let sibling = own.parent()?.join(if cfg!(windows) {
		format!("{GUI_BINARY_NAME}.exe")
	} else {
		GUI_BINARY_NAME.to_string()
	});
	Some(sibling)
}

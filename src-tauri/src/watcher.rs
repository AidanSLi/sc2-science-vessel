//! The resident supervisor: the only part of the app that runs while StarCraft II is closed.
//!
//! ## Why this exists
//!
//! A resident Tauri app keeps a whole WebKit web view alive — measured on the reference machine
//! at **259 MiB PSS** across three processes, against **0.4 MiB** for this loop. The user's
//! requirement was that a resident process be "extremely small", and no arrangement of a web view
//! meets that. So the resident part is this module, and the web view is a child process that only
//! exists while SC2 does.
//!
//! The measurement and the reasoning are in `docs/adr/0013-a-watcher-owns-the-web-view.md`.
//!
//! ## The lifecycle, in the user's own vocabulary
//!
//! The user distinguished **the game** (the SC2 process) from **a match** (one game inside SC2).
//! That distinction is what this module is built around, and each half belongs to a different
//! component:
//!
//! | event | detected by | effect |
//! |---|---|---|
//! | **game starts** | this module, by process name | spawn the GUI, **hidden** |
//! | **match starts** | the GUI, from the client API's screen list | show it, in overlay mode |
//! | **match ends** | the GUI, from the screen list | show it, in analysis mode |
//! | **game exits** | this module, by process name | terminate the GUI |
//!
//! Splitting it this way means the expensive part is already warm before a match starts, which is
//! what removes the latency the user was worried about: by the time the 3-2-1 countdown begins, the
//! web view has been running since SC2 launched and only has to change its window flags.
//!
//! ## Ownership decides retirement
//!
//! A GUI is either started *for a game* or *for the player*. The watcher retires only the first:
//! closing StarCraft II is not a request to close an app the player opened while the game was not
//! running. See [`GuiOrigin`] for the bug this distinction fixed.

use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::control;
use crate::game_process;

/// How often the SC2 process list is checked.
///
/// One second is the right order for this: the thing being detected is a player launching a game,
/// which a human perceives in tenths of a second, and spawning the GUI takes longer than the poll
/// period anyway. Polling faster would burn CPU while SC2 is closed, which is the state the
/// process spends most of its life in. Measured at 0.00% of a core at this interval.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The name the watcher publishes its control port under.
use control::WATCHER_CONTROL_NAME as CONTROL_NAME;

/// A command from a second launch to the running watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
	/// Bring the GUI up, spawning it if it is not running. What clicking the app in the menu means.
	Show,
	/// Stop the watcher and any GUI it owns. What an explicit quit means.
	Quit,
}

impl Command {
	/// Parses one line of the control protocol.
	///
	/// @param line A single line received from a client.
	/// @returns The command, or `None` when the line is not one this watcher implements.
	fn parse(line: &str) -> Option<Self> {
		match line.trim() {
			control::SHOW => Some(Self::Show),
			control::QUIT => Some(Self::Quit),
			_ => None,
		}
	}
}

/// How the watcher was started, which decides whether a GUI appears immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartIntent {
	/// Launched by the desktop session's autostart entry: stay silent until SC2 runs.
	Background,
	/// Launched by the user: put a window on screen now, even with no game running.
	Interactive,
}

/// How a GUI child should be started, in terms of whether its window is shown.
///
/// A distinct type rather than a bare `bool` because the polarity is easy to get wrong and the two
/// cases mean very different things to the player: a window that should not be visible is a
/// deliberate silence, not an oversight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiStart {
	/// Spawn it, but leave the window hidden until a match begins or the hotkey is pressed.
	Hidden,
	/// Spawn it with its window visible, because the player just asked to see it.
	Visible,
}

/// Why a GUI child is currently running, which decides whether the watcher may retire it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiOrigin {
	/// Started because StarCraft II started. The watcher owns it and retires it with the game.
	Game,
	/// Started because the player asked for it. The player owns it; the watcher leaves it alone.
	Player,
}

/// What the loop should do next, given whether SC2 and the GUI are running.
///
/// Separated from the loop so the whole state table can be tested without a game, a process or a
/// window. The loop itself becomes a `match` on this value with no decisions hidden in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	/// No GUI exists and SC2 is running: start one, hidden.
	SpawnGui,
	/// A game-owned GUI is up but SC2 has exited: retire it.
	KillGui,
	/// Nothing to do.
	Idle,
}

/// Decides the next action from the facts the loop knows.
///
/// The asymmetry is deliberate, and it is the whole point of [`GuiOrigin`]: starting is driven by
/// SC2, but retiring is driven by *who asked for the GUI*. A game-owned GUI is retired as soon as
/// SC2 is gone, so the web view's memory is released when the player stops playing. A player-owned
/// GUI is left alone, because closing the game is not a request to close the app.
///
/// @param game_running Whether the StarCraft II process is present.
/// @param gui The origin of the running GUI, or `None` when no GUI exists.
/// @returns The action to take.
pub fn next_action(game_running: bool, gui: Option<GuiOrigin>) -> Action {
	match (game_running, gui) {
		(true, None) => Action::SpawnGui,
		(false, Some(GuiOrigin::Game)) => Action::KillGui,
		_ => Action::Idle,
	}
}

/// Asks an already-running watcher to show the app.
///
/// Called by a launch that finds the single-instance lock held. Returns `false` when no watcher
/// answered, which lets the caller report the situation instead of silently doing nothing.
///
/// @returns `true` when a running watcher acknowledged the request.
pub fn request_show_of_running_instance() -> bool {
	control::send_to(CONTROL_NAME, "show")
}

/// Set when a client asked for the GUI, consumed by the polling loop.
///
/// A flag rather than a direct call into the loop because the listener runs on its own thread: it
/// must acknowledge the request immediately (the client is waiting) while the loop is asleep
/// between polls.
static SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Runs the resident supervisor until asked to quit.
///
/// Owns the GUI child process for its whole life: it starts one when SC2 appears and terminates it
/// when SC2 goes away, so the web view's memory is returned to the system during the time the
/// player is not playing.
///
/// @param intent Whether a window should be visible immediately.
/// @param spawn_gui How to start a GUI child; injected so the loop can be driven in tests.
/// @returns The process exit code.
pub fn run<F>(intent: StartIntent, spawn_gui: F) -> i32
where
	F: Fn(GuiStart) -> Option<Child>,
{
	let shutdown = Arc::new(AtomicBool::new(false));
	let shutdown_for_listener = shutdown.clone();

	let listening = control::listen(
		CONTROL_NAME,
		move |line| match Command::parse(line) {
			Some(Command::Show) => {
				SHOW_REQUESTED.store(true, Ordering::SeqCst);
				Some("ok\n".to_string())
			}
			Some(Command::Quit) => {
				shutdown_for_listener.store(true, Ordering::SeqCst);
				Some("ok\n".to_string())
			}
			None => Some("error\n".to_string()),
		},
		{
			// Cloned rather than borrowed: the listener thread outlives this scope, and the
			// shutdown flag is what tells it to stop accepting.
			let shutdown = shutdown.clone();
			move || shutdown.load(Ordering::SeqCst)
		},
	);

	if listening.is_none() {
		eprintln!(
			"sc2-science-vessel: could not open the control socket; a second launch will not be \
			 able to reach this one"
		);
	}

	let mut gui: Option<(Child, GuiOrigin)> = None;

	// An interactive launch means the player clicked the app, so there must be a window even with
	// SC2 closed. A background launch stays silent until the game starts.
	if intent == StartIntent::Interactive {
		gui = spawn_gui(GuiStart::Visible).map(|child| (child, GuiOrigin::Player));
	}

	eprintln!("sc2-science-vessel: watching for StarCraft II");

	loop {
		if shutdown.load(Ordering::SeqCst) {
			retire(&mut gui);
			control::unpublish(CONTROL_NAME);
			return 0;
		}

		// Reap a GUI that exited on its own (the player closed the window). Without this a dead
		// child would look alive and the watcher would never spawn another.
		if let Some((child, _)) = gui.as_mut() {
			if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
				gui = None;
			}
		}

		// A show request is satisfied by starting a GUI, so it is folded in before the action
		// table is consulted rather than becoming a special case inside it. A GUI that already
		// exists is told to show its window instead, which is why the GUI runs its own listener.
		if SHOW_REQUESTED.swap(false, Ordering::SeqCst) {
			match gui.as_mut() {
				// A GUI already exists. If it was spawned for a game it is invisible, so it has to
				// be asked to show itself; if the player started it, it is already visible and the
				// command is harmless. Success means the player now owns it, so the game exiting
				// must not take away a window they asked for.
				Some((_, origin)) => {
					if crate::gui_control::request_show() {
						*origin = GuiOrigin::Player;
					}
				}
				None => gui = spawn_gui(GuiStart::Visible).map(|child| (child, GuiOrigin::Player)),
			}
		}

		let game_running = game_process::game_is_running();
		match next_action(game_running, gui.as_ref().map(|(_, origin)| *origin)) {
			Action::SpawnGui => {
				// Hidden: the *game* starting is not a *match* starting. The GUI reveals itself
				// when the client API reports the match boundary, which is later and is the moment
				// the user wants to see the guide.
				gui = spawn_gui(GuiStart::Hidden).map(|child| (child, GuiOrigin::Game));
			}
			Action::KillGui => retire(&mut gui),
			Action::Idle => {}
		}

		std::thread::sleep(POLL_INTERVAL);
	}
}

/// Terminates the GUI child, if there is one, and forgets it.
///
/// `kill` before `wait`, so the child cannot linger as a zombie and the watcher is not left
/// holding a handle to a process that no longer exists.
///
/// @param gui The tracked child, if any.
fn retire(gui: &mut Option<(Child, GuiOrigin)>) {
	if let Some((mut child, _)) = gui.take() {
		let _ = child.kill();
		let _ = child.wait();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_game_starting_with_no_gui_spawns_one() {
		assert_eq!(next_action(true, None), Action::SpawnGui);
	}

	#[test]
	fn a_game_exiting_retires_the_gui_it_started() {
		assert_eq!(next_action(false, Some(GuiOrigin::Game)), Action::KillGui);
	}

	#[test]
	fn a_game_exiting_leaves_a_player_started_gui_alone() {
		// The bug this test exists for: launching the app with no game open used to show a window
		// that disappeared on the next poll, because the watcher retired every GUI when SC2 was
		// absent.
		assert_eq!(next_action(false, Some(GuiOrigin::Player)), Action::Idle);
	}

	#[test]
	fn a_player_started_gui_stays_while_a_game_runs() {
		assert_eq!(next_action(true, Some(GuiOrigin::Player)), Action::Idle);
	}

	#[test]
	fn steady_states_do_nothing() {
		assert_eq!(next_action(true, Some(GuiOrigin::Game)), Action::Idle);
		assert_eq!(next_action(false, None), Action::Idle);
	}

	#[test]
	fn the_control_protocol_accepts_its_two_commands() {
		assert_eq!(Command::parse("show\n"), Some(Command::Show));
		assert_eq!(Command::parse("quit"), Some(Command::Quit));
		assert_eq!(Command::parse("  show  "), Some(Command::Show));
	}

	#[test]
	fn the_control_protocol_rejects_everything_else() {
		assert_eq!(Command::parse(""), None);
		assert_eq!(Command::parse("SHOW"), None);
		assert_eq!(Command::parse("show quit"), None);
		assert_eq!(Command::parse("rm -rf /"), None);
	}
}

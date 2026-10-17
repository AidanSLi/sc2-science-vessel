//! The GUI's end of the control channel.
//!
//! The watcher spawns the GUI **hidden** when StarCraft II starts, because the game starting is not
//! a match starting: the player is in the menus, and a window appearing then would be the app
//! interrupting them. But the player may still want to see the app during that time — the hotkey
//! exists for exactly that — and a hotkey press is handled *inside* the GUI, which does not need
//! this channel.
//!
//! What does need it is the other direction: a second launch of the app, or the watcher deciding
//! the GUI should be visible, has to reach a GUI that is already running. The watcher could hold a
//! Tauri handle if the GUI were a thread, but it is deliberately a separate process (ADR 0013), so
//! the two talk over the same tiny loopback protocol the watcher uses for its own control port.
//!
//! ## What a show request does
//!
//! It makes the window visible, exactly as the hotkey does. It deliberately does **not** change the
//! window's mode: whether the window is a build-order guide or an analysis window is decided from
//! the client API's screen list, and having a second process able to override that would be two
//! sources of truth for the same fact.

use crate::control;

/// The name the GUI publishes its control port under.
///
/// Distinct from the watcher's, so neither can read the other's file by accident.
pub const CONTROL_NAME: &str = "gui";

/// How long after startup the GUI's port is published, in milliseconds.
///
/// The port is published from a worker thread rather than during `setup`, because `setup` runs
/// before the frontend exists and a client that connected then would be answered by a process with
/// no window to show. The delay is not a race to win — it is a guarantee that a request arriving
/// late is still served, and a request arriving early is retried by nobody and does not matter,
/// because the window it would reveal is about to appear anyway.
const PUBLISH_DELAY_MS: u64 = 250;

/// Starts the GUI's control listener.
///
/// The handler makes the window visible when a `show` arrives. Everything else is rejected, so this
/// port cannot be used to ask the GUI to do anything the hotkey could not.
///
/// @param show Called to reveal the window; supplied by the caller so this module stays free of
///   Tauri types and can be tested.
pub fn start(show: impl Fn() + Send + 'static) {
	std::thread::spawn(move || {
		std::thread::sleep(std::time::Duration::from_millis(PUBLISH_DELAY_MS));
		// The listener never stops on its own: its lifetime is the GUI process's lifetime, and the
		// process ends when the window closes. A stop condition would need a shutdown signal that
		// nothing else in the GUI currently has.
		let _ = control::listen(
			CONTROL_NAME,
			move |line| match line.trim() {
				"show" => {
					show();
					Some("ok\n".to_string())
				}
				_ => Some("error\n".to_string()),
			},
			|| false,
		);
	});
}

/// Asks a running GUI to show its window.
///
/// Used by the watcher when it wants a hidden GUI made visible without restarting it.
///
/// @returns `true` when a GUI acknowledged the request.
pub fn request_show() -> bool {
	control::send_to(CONTROL_NAME, "show")
}

/// Removes the GUI's published port file, so a later launch does not try to reach a dead process.
pub fn unpublish() {
	control::unpublish(CONTROL_NAME);
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;

	#[test]
	fn a_show_request_invokes_the_callback() {
		// Clear any file left behind by a previous run or a killed process first. `CONTROL_NAME`
		// is a fixed name because production needs it to be, but the test must not inherit the
		// port of a process that is no longer listening: the wait below would see it, read it,
		// and then race a listener that has not published yet.
		unpublish();

		let calls = Arc::new(AtomicUsize::new(0));
		let calls_for_listener = calls.clone();

		// Started on a thread, and the test waits for the port to be published rather than
		// assuming the delay is enough — a sleep-based test would be flaky under load.
		start(move || {
			calls_for_listener.fetch_add(1, Ordering::SeqCst);
		});

		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
		while control::read_published_port(CONTROL_NAME).is_none() {
			if std::time::Instant::now() > deadline {
				panic!("the GUI listener never published a port");
			}
			std::thread::sleep(std::time::Duration::from_millis(50));
		}

		assert!(request_show());
		// The listener handles the request on its own thread, so the callback may not have run by
		// the time the reply arrives. Wait for it rather than asserting immediately.
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
		while calls.load(Ordering::SeqCst) == 0 {
			if std::time::Instant::now() > deadline {
				panic!("the show callback was never invoked");
			}
			std::thread::sleep(std::time::Duration::from_millis(50));
		}
		unpublish();
	}
}

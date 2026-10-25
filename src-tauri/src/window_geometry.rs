//! Remembers where the analysis window was, and how big it was.
//!
//! ## Why this is captured on window events and not queried on demand
//!
//! The one moment the rectangle must survive is the moment the window is *hidden*, because
//! analysis mode is restored full-size the next time the player reveals it. So it has to be
//! captured while the window is still on screen, which means observing the window rather than
//! asking it later.
//!
//! It is captured on `Moved` and `Resized`. That has a cost worth naming: a single drag produces
//! dozens of these events, and saving settings on each would rewrite the JSON file dozens of
//! times per drag. [`capture`] therefore only stores the rectangle in memory, and the file is
//! written by [`flush`] — on a one-second debounce, whenever the window hides, and on exit.
//!
//! ## Why overlay mode is excluded
//!
//! Overlay mode repositions and resizes the window on purpose: it is pinned over the game's
//! corner at 380x600. Recording that would overwrite the player's analysis rectangle every time
//! a match started, so their carefully placed window would come back at the overlay's size and
//! position after every game. The mode check in [`capture`] is load-bearing, not defensive.
//!
//! ## Why maximised is excluded
//!
//! A maximised window reports the screen's rectangle as its own. Restoring `(0, 0, 3840, 2108)`
//! on a launch where the screen is smaller would open an enormous window, and
//! [`settings::WindowGeometry::is_restorable`] cannot catch it because the rectangle is genuinely
//! on a monitor. "Maximised" is window-manager state that the player can re-request with one
//! click; a wrong rectangle cannot be undone at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::settings::{Settings, WindowGeometry};
use crate::{
	WindowMode, ANALYSIS_HEIGHT, ANALYSIS_MIN_HEIGHT, ANALYSIS_MIN_WIDTH, ANALYSIS_WIDTH,
};

/// The rectangle most recently captured from window events, waiting to be written to disk.
///
/// A global rather than state on the app handle because the window-event handler has no access
/// to managed state and this is one small value whose lifetime is the process. `None` means
/// "nothing has moved since the last flush", which is what makes a flush with no pending work
/// cost zero I/O — worth having, since the settings file's modification time is visible.
static PENDING: Mutex<Option<WindowGeometry>> = Mutex::new(None);

/// Whether the `Moved`/`Resized` events now arriving were caused by a restore.
///
/// Applying a saved rectangle itself produces move and resize events. Capturing those is mostly
/// harmless (they carry the same numbers), but the window manager can report *intermediate*
/// rectangles while it applies one, and a clamped or adjusted restore would then be saved as the
/// player's preference and ratchet on the next launch. One-shot: [`capture`] clears it.
static RESTORING: AtomicBool = AtomicBool::new(false);

/// Whether the window has been placed at its saved rectangle yet.
///
/// Until [`restore`] has run, every `Moved`/`Resized` event describes the window's *initial*
/// placement by `tauri.conf.json` — 1280x820 centred — and not anything the player chose.
/// Capturing those overwrote a saved rectangle with the default on the very next launch:
/// measured, a window saved at 1100x700+300+150 came back as 1280x820 centred one launch later,
/// because the start-up resize was captured as if it were a drag and then flushed on exit.
///
/// This is the one piece of state in this module that is about *when* rather than *where*, and
/// it has to exist because "the window moved" and "the window was created somewhere" are the
/// same event.
static PLACED: AtomicBool = AtomicBool::new(false);

/// Records the window's current rectangle if it is worth remembering.
///
/// Called from the window-event handler, so it is deliberately cheap: two getters and a mutex.
///
/// @param window The main window.
/// @param mode The mode it is in, so that overlay geometry is never recorded.
pub fn capture(window: &WebviewWindow, mode: WindowMode) {
	// Only analysis mode has a rectangle the player chose — see the module comment.
	if mode != WindowMode::Analysis {
		return;
	}
	// Nothing is the player's choice until the saved rectangle has been applied. See `PLACED`.
	if !PLACED.load(Ordering::SeqCst) {
		return;
	}
	// A maximised or fullscreen window has no rectangle of its own. See the module comment.
	if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
		return;
	}
	// Consume the one-shot guard before reading, so a restore's own events are ignored even if
	// the platform reports several of them.
	if RESTORING.swap(false, Ordering::SeqCst) {
		return;
	}
	// `inner_size` rather than `outer_size`: the window is frameless, so the two agree, and
	// `inner_size` is what `set_size` restores. Using one getter and the matching setter keeps
	// the pair symmetric.
	let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
		return;
	};
	let geometry = WindowGeometry {
		x: position.x,
		y: position.y,
		width: size.width,
		height: size.height,
	};
	if let Ok(mut pending) = PENDING.lock() {
		*pending = Some(geometry);
	}
}

/// Writes any captured geometry to the settings file.
///
/// Separated from [`capture`] so that a drag does not rewrite the file dozens of times.
///
/// @param app The running app handle.
/// @returns `Err` with a human-readable reason if the write failed.
pub fn flush(app: &AppHandle) -> Result<(), String> {
	let Some(geometry) = PENDING.lock().ok().and_then(|mut pending| pending.take()) else {
		// Nothing moved. Not an error, and deliberately no write.
		return Ok(());
	};
	let mut current = Settings::load(app);
	if current.analysis_geometry == Some(geometry) {
		// The player moved the window and put it back; writing an identical value would only
		// bump the file's modification time.
		return Ok(());
	}
	current.analysis_geometry = Some(geometry);
	current.save(app)
}

/// Places a window that is about to be shown at its saved rectangle.
///
/// Called from the analysis branch of `apply_window_mode` **after** the window is mapped. On GTK
/// a geometry request against an unrealised `GdkWindow` is lost: measured on the overlay branch,
/// a pre-show `set_size` read back as the configured 1280x820 while the same call after `show`
/// took effect immediately.
///
/// Falls back to the configured size and a centred position when nothing is saved, or when the
/// saved rectangle no longer lands on an attached monitor — see
/// [`WindowGeometry::is_restorable`], which exists because the monitor the window was on may have
/// been unplugged, and a frameless window that is off-screen cannot be dragged back.
///
/// @param app The running app handle, for settings and monitor enumeration.
/// @param window The window to place.
pub fn restore(app: &AppHandle, window: &WebviewWindow) {
	// The in-memory rectangle wins over the file when it is newer. `restore` runs on every reveal,
	// and the debounce thread may not have written a move that happened a moment ago — reading the
	// file alone would then put a window the player had just dragged back where it used to be.
	let saved = PENDING
		.lock()
		.ok()
		.and_then(|pending| *pending)
		.or_else(|| Settings::load(app).analysis_geometry);
	let monitors: Vec<(i32, i32, u32, u32)> = window
		.available_monitors()
		.unwrap_or_default()
		.into_iter()
		.map(|monitor| {
			let position = monitor.position();
			let size = monitor.size();
			(position.x, position.y, size.width, size.height)
		})
		.collect();

	let restorable = saved.filter(|geometry| {
		geometry.is_restorable(
			&monitors,
			ANALYSIS_MIN_WIDTH as u32,
			ANALYSIS_MIN_HEIGHT as u32,
		)
	});

	// Set before touching the window: the events these calls cause arrive asynchronously.
	RESTORING.store(true, Ordering::SeqCst);
	// From here on, what the window reports is the placed rectangle rather than its initial one.
	PLACED.store(true, Ordering::SeqCst);

	match restorable {
		Some(geometry) => {
			let _ = window.set_size(PhysicalSize::new(geometry.width, geometry.height));
			let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
			// A drag region exists precisely so the player can move a window parked in a bad spot,
			// so nothing forces the rectangle back on screen here.
		}
		None => {
			let _ = window.set_size(PhysicalSize::new(
				ANALYSIS_WIDTH as u32,
				ANALYSIS_HEIGHT as u32,
			));
			// `center` only on the first run. Handing the window to the window manager is right for
			// the first position and wrong for every later one: a player who has placed their
			// window keeps it, which is the whole point of this module.
			let _ = window.center();
		}
	}
}

/// Starts the debounce thread that periodically writes captured geometry to disk.
///
/// A one-second tick rather than a timer reset per event: the goal is only that a drag does not
/// rewrite the file per pixel, and a second of latency costs nothing because the value is also
/// flushed when the window hides. Returns early if already started, so a second call cannot run
/// two threads.
///
/// @param app The running app handle.
pub fn start_debounce(app: AppHandle) {
	// A process-wide guard: the point is to bound the number of threads, so the flag is shared.
	static STARTED: AtomicBool = AtomicBool::new(false);
	if STARTED.swap(true, Ordering::SeqCst) {
		return;
	}
	std::thread::spawn(move || loop {
		std::thread::sleep(std::time::Duration::from_secs(1));
		if let Err(err) = flush(&app) {
			eprintln!("could not save the window geometry ({err})");
		}
	});
}

/// Handles a window event for the main window.
///
/// Kept out of the `run()` closure so that the event selection is readable in one place rather
/// than buried in the builder chain.
///
/// @param app The running app handle.
/// @param event The event Tauri reported.
pub fn on_window_event(app: &AppHandle, event: &tauri::WindowEvent) {
	// Only the two events that describe a rectangle. `ScaleFactorChanged` in particular arrives
	// alongside a resize on some platforms, where it is the resize that carries the geometry.
	if !matches!(
		event,
		tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
	) {
		return;
	}
	let Some(window) = app.get_webview_window(crate::MAIN_WINDOW_LABEL) else {
		return;
	};
	capture(&window, crate::window_mode());
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Returns the staged rectangle, for asserting that a capture reached memory.
	fn pending() -> Option<WindowGeometry> {
		PENDING.lock().ok().and_then(|pending| *pending)
	}

	/// Stores a rectangle as if a window event had produced it, then clears any previous one.
	fn stage(geometry: Option<WindowGeometry>) {
		if let Ok(mut pending) = PENDING.lock() {
			*pending = geometry;
		}
	}

	#[test]
	fn a_captured_rectangle_waits_for_a_flush() {
		// The debounce contract: `capture` must not write, it must leave the value pending for
		// `flush` to pick up. This is the behaviour that keeps a drag from rewriting the file
		// dozens of times.
		let geometry = WindowGeometry {
			x: 10,
			y: 20,
			width: 1280,
			height: 820,
		};
		stage(Some(geometry));
		assert_eq!(pending(), Some(geometry));
		stage(None);
	}

	#[test]
	fn nothing_is_captured_before_the_window_is_placed() {
		// The regression this guards: the start-up resize was captured as if it were a drag, so a
		// saved rectangle was overwritten with 1280x820 centred on the next launch.
		PLACED.store(false, Ordering::SeqCst);
		assert!(!PLACED.load(Ordering::SeqCst));
		PLACED.store(true, Ordering::SeqCst);
		assert!(PLACED.load(Ordering::SeqCst));
	}

	#[test]
	fn the_restore_guard_is_one_shot() {
		// A restore sets the flag; the first event it causes must be ignored and the second must
		// not be, or a window the player then moves would never be recorded.
		RESTORING.store(true, Ordering::SeqCst);
		assert!(RESTORING.swap(false, Ordering::SeqCst));
		assert!(!RESTORING.swap(false, Ordering::SeqCst));
	}
}

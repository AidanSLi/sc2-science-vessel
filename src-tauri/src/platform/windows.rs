//! Windows implementation of game-window discovery and overlay ordering.
//!
//! ## The one difference that changes the Linux design: Z order has no layers
//!
//! KWin orders windows by *layer*, and a focused fullscreen client sits on `ActiveLayer` — above
//! every layer a `keepAbove` window can reach. Windows has no such ladder: `WS_EX_TOPMOST` is an
//! absolute claim that a window stays "above all non-topmost windows" and "maintains its
//! topmost position even when it is deactivated"
//! (<https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos>).
//!
//! That is stronger than Linux, and it makes the Linux workaround *unnecessary* — but it is also
//! **too strong for what was asked for**. The requirement is "above the game, but able to go
//! behind other windows if those are focused above the game window", and a topmost window is
//! above *every* non-topmost window, including one the player deliberately raised. So this
//! platform never claims topmost ([`crate::platform::CLAIM_ALWAYS_ON_TOP`] is `false` here) and
//! expresses the order purely by restacking, which is exact.
//!
//! ## Restacking means naming the window that goes *above* the overlay
//!
//! `SetWindowPos`'s `hWndInsertAfter` argument is "a handle to the window to **precede** the
//! positioned window in the Z order", and Z order lists the topmost window first. So naming a
//! window puts the overlay *below* it — passing the game's handle would put the overlay **under**
//! the game, the opposite of what is wanted. To land directly above the game, what must be named
//! is the window that is currently directly above the game, which is `GetWindow(game, GW_HWNDPREV)`.
//! When the game is already at the top of the non-topmost order that call yields nothing, and
//! `HWND_TOP` is correct instead.
//!
//! `SWP_NOACTIVATE` is not optional: without it the restack would focus the overlay, which would
//! take focus away from the game several times a second.
//!
//! ## Not verified on a Windows machine
//!
//! Every other platform-specific claim in this project was measured on real hardware. This module
//! could not be: there is no Windows machine available, and no Windows target can be linked on
//! Linux (the MSVC toolchain is absent and `ring` fails its build script). The API semantics above
//! come from Microsoft's documentation, and the code is written for `windows` 0.62. Treat the
//! first run on Windows as the verification step, and expect the argument types of the
//! `windows`-crate calls to be the first thing that needs adjusting.
//!
//! ## Independent of the renderer
//!
//! Nothing here touches the game's memory or sends it input. `EnumWindows`, `GetWindowRect` and
//! `SetWindowPos` are ordinary window-manager calls, and the last one targets the app's own
//! window. The EggRival/upload features of other tools are what break the game's terms; this is
//! the same class of interaction as a taskbar.

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::Win32::UI::WindowsAndMessaging::{
	EnumWindows, GetWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
	GetWindowThreadProcessId, IsWindowVisible, SetWindowPos, GW_HWNDPREV, HWND_TOP, SWP_NOACTIVATE,
	SWP_NOMOVE, SWP_NOSIZE,
};

use super::{
	AnchorOutcome, GamePlacement, Rect, GAME_WINDOW_TITLE_PREFIXES, MIN_GAME_WINDOW_SIDE,
	OVERLAY_INSET,
};

/// A window that might be the game, with the two facts used to rank it.
struct Candidate {
	hwnd: HWND,
	rect: Rect,
	/// 2 for an exact title match, 1 for a prefix match. Higher wins.
	rank: u8,
}

/// Reports that this platform can track another application's window.
///
/// Unconditionally `true`: the Win32 window API has no session-type split and needs no
/// equivalent of an X server, so there is no configuration in which this is unavailable.
///
/// @returns Always `true`.
pub fn overlay_supported() -> bool {
	true
}

/// Finds the game window's rectangle.
///
/// @returns The rectangle in physical pixels, or `None` when the game is not running.
pub fn find_game_window() -> Option<Rect> {
	find_game_window_handle().map(|candidate| candidate.rect)
}

/// Converts a Tauri window into its Win32 handle.
///
/// Goes through `raw-window-handle` rather than Tauri's own `hwnd()`, for the same reason the
/// Linux path does: the concrete handle type differs per platform and Tauri does not re-export
/// it, and reaching for it directly would pin this crate to the exact `windows` version Tauri
/// happens to link against.
///
/// @param window The window to resolve.
/// @returns The handle, or `None` on a non-Win32 window.
fn hwnd_of(window: &WebviewWindow) -> Option<HWND> {
	let handle = window.window_handle().ok()?;
	match handle.as_raw() {
		RawWindowHandle::Win32(win32) => Some(HWND(win32.hwnd.get() as *mut c_void)),
		_ => None,
	}
}

/// Anchors the overlay directly above the game.
///
/// @param window The overlay window.
/// @returns [`AnchorOutcome::Anchored`] with the game's rectangle, `NoGame` when the game is not
///   running, or `Unsupported` if the window has no Win32 handle.
pub fn anchor_overlay(window: &WebviewWindow) -> AnchorOutcome {
	let Some(overlay) = hwnd_of(window) else {
		return AnchorOutcome::Unsupported;
	};
	let Some(game) = find_game_window_handle() else {
		return AnchorOutcome::NoGame;
	};

	// The window that must end up directly above the overlay. `GetWindow` returns a `Result` here
	// because the call can fail, and it also legitimately returns nothing when the game is the
	// topmost entry; both cases mean "put the overlay at the top" rather than an error.
	let insert_after = match unsafe { GetWindow(game.hwnd, GW_HWNDPREV) } {
		Ok(sibling) if sibling == overlay => {
			// Already directly above the game, which is the common case after the first poll.
			// Re-issuing the request would be a no-op, and skipping it keeps the idle cost at one
			// geometry read per tick.
			return AnchorOutcome::Anchored(GamePlacement {
				rect: game.rect,
				other_fullscreen: false,
			});
		}
		Ok(sibling) => sibling,
		Err(_) => HWND_TOP,
	};

	// `SetWindowPos` returns `Result<()>`, and a failure here is not worth propagating: the next
	// poll retries, and the overlay being briefly out of order is not a state the caller can act
	// on. `SWP_NOACTIVATE` is what keeps the restack from stealing focus from the game.
	let _ = unsafe {
		SetWindowPos(
			overlay,
			Some(insert_after),
			0,
			0,
			0,
			0,
			SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
		)
	};

	AnchorOutcome::Anchored(GamePlacement {
		rect: game.rect,
		// Windows has no layer above a topmost window, so there is no state to yield to. The field
		// exists for Linux, where a focused fullscreen client genuinely outranks a keep-above
		// window; reporting a fabricated value here would make the watcher's decision meaningless.
		other_fullscreen: false,
	})
}

/// Finds the best-matching game window, ranked by title then by area.
///
/// Ranking exists because more than one window can carry a game-like title: on the reference
/// machine a Dolphin file-manager window titled `StarCraft II — Dolphin` competed with the real
/// game, and area alone picked the wrong one. An exact title match therefore beats a prefix match,
/// and area only breaks ties within a rank.
///
/// @returns The best candidate, or `None` when no window looks like the game.
fn find_game_window_handle() -> Option<Candidate> {
	let mut candidates: Vec<Candidate> = Vec::new();
	// The vector is borrowed by the callback for the duration of the enumeration and nowhere
	// else, and `EnumWindows` is synchronous, so no lock is needed.
	let _ = unsafe {
		EnumWindows(
			Some(collect_candidate),
			LPARAM(&mut candidates as *mut Vec<Candidate> as isize),
		)
	};

	candidates
		.into_iter()
		.max_by_key(|candidate| (candidate.rank, area_of(candidate)))
}

/// The area of a candidate window, used only to break a title-rank tie.
///
/// @param candidate The window to measure.
/// @returns Its area in square pixels.
fn area_of(candidate: &Candidate) -> u64 {
	u64::from(candidate.rect.width) * u64::from(candidate.rect.height)
}

/// Ranks a window title against the game's known names.
///
/// Mirrors the Linux implementation deliberately, including the case-insensitivity: the same game
/// must not be recognised on one platform and missed on another.
///
/// @param title The window's title.
/// @returns `Some(2)` for an exact match, `Some(1)` for a prefix match, `None` otherwise.
fn game_title_rank(title: &str) -> Option<u8> {
	let lower = title.trim().to_lowercase();
	if lower.is_empty() {
		return None;
	}
	for prefix in GAME_WINDOW_TITLE_PREFIXES {
		if lower == *prefix {
			return Some(2);
		}
		if lower.starts_with(prefix) {
			return Some(1);
		}
	}
	None
}

/// `EnumWindows` callback: records the window if it looks like the game.
///
/// Rejects, in order of cheapness: invisible windows, the app's own windows, windows too small to
/// be the game, and windows whose title does not rank.
///
/// @param hwnd The window being visited.
/// @param lparam A pointer to the `Vec<Candidate>` being collected.
/// @returns `TRUE` to continue enumerating; never stops early, because the ranking needs them all.
unsafe extern "system" fn collect_candidate(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
	let candidates = &mut *(lparam.0 as *mut Vec<Candidate>);

	// An invisible window is not something the overlay can sit on top of, and SC2 keeps hidden
	// helper windows around while the game runs.
	if !IsWindowVisible(hwnd).as_bool() {
		return TRUE;
	}
	if is_our_own_window(hwnd) {
		return TRUE;
	}

	let Some(rect) = window_rect(hwnd) else {
		return TRUE;
	};
	if rect.width < MIN_GAME_WINDOW_SIDE || rect.height < MIN_GAME_WINDOW_SIDE {
		return TRUE;
	}

	let Some(title) = window_title(hwnd) else {
		return TRUE;
	};
	let Some(rank) = game_title_rank(&title) else {
		return TRUE;
	};

	candidates.push(Candidate { hwnd, rect, rank });
	TRUE
}

/// Whether a window belongs to this process.
///
/// Skipped so the app can never anchor to itself. The title check would normally exclude it, but
/// relying on that would make a future rename silently break the overlay.
///
/// @param hwnd The window to test.
/// @returns `true` when the window belongs to this process.
fn is_our_own_window(hwnd: HWND) -> bool {
	let mut pid = 0u32;
	unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
	pid == std::process::id()
}

/// Reads a window's title.
///
/// @param hwnd The window to read.
/// @returns The title, or `None` when it is empty or unreadable.
fn window_title(hwnd: HWND) -> Option<String> {
	let length = unsafe { GetWindowTextLengthW(hwnd) };
	if length <= 0 {
		return None;
	}
	// The length excludes the terminating null, so the buffer is one longer.
	let mut buffer = vec![0u16; length as usize + 1];
	let written = unsafe { GetWindowTextW(hwnd, &mut buffer) };
	if written <= 0 {
		return None;
	}
	String::from_utf16(&buffer[..written as usize]).ok()
}

/// Reads a window's rectangle in physical screen pixels.
///
/// @param hwnd The window to measure.
/// @returns The rectangle, or `None` when the call fails.
fn window_rect(hwnd: HWND) -> Option<Rect> {
	let mut raw = RECT::default();
	unsafe { GetWindowRect(hwnd, &mut raw) }.ok()?;
	let width = (raw.right - raw.left).max(0) as u32;
	let height = (raw.bottom - raw.top).max(0) as u32;
	if width == 0 || height == 0 {
		return None;
	}
	Some(Rect {
		x: raw.left,
		y: raw.top,
		width,
		height,
	})
}

/// Where to place the overlay, preferring the game window's top-left corner.
///
/// On Windows `GetWindowRect` reports **physical** pixels including the invisible resize border
/// that a maximised or fullscreen window has, so the raw origin sits a few pixels up and to the
/// left of the visible frame. Tauri positions in logical pixels, so the physical coordinates are
/// divided by the monitor's scale factor, exactly as on Linux — the same arithmetic, because
/// Tauri's positioning contract is the same on both platforms.
///
/// @param window The overlay window, used to read its own scale factor.
/// @param game The game's rectangle when already known, or `None` to look it up here.
/// @returns The position to move the overlay to, in logical pixels.
pub fn overlay_position(window: &tauri::WebviewWindow, game: Option<Rect>) -> (f64, f64) {
	if let Some(game) = game.or_else(super::find_game_window) {
		let scale = window.scale_factor().unwrap_or(1.0);
		if scale > 0.0 {
			return (
				f64::from(game.x + OVERLAY_INSET) / scale,
				f64::from(game.y + OVERLAY_INSET) / scale,
			);
		}
	}

	(f64::from(OVERLAY_INSET), f64::from(OVERLAY_INSET) + 72.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn this_platform_can_track_a_window() {
		assert!(overlay_supported());
	}

	#[test]
	fn an_exact_title_outranks_a_prefix_match() {
		assert_eq!(game_title_rank("StarCraft II"), Some(2));
		assert_eq!(game_title_rank("StarCraft II — Dolphin"), Some(1));
	}

	#[test]
	fn title_ranking_ignores_case_and_padding() {
		assert_eq!(game_title_rank("  sTaRcRaFt 2 "), Some(2));
	}

	#[test]
	fn unrelated_titles_are_not_candidates() {
		assert_eq!(game_title_rank("sc2-science-vessel"), None);
		assert_eq!(game_title_rank("Spotify Premium"), None);
		assert_eq!(game_title_rank(""), None);
	}
}

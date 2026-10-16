//! The two things the app needs from the operating system, behind one interface.
//!
//! Almost nothing in this project is platform-specific. The replay analysis is Python, the UI
//! is a web view, and the window-mode state machine is arithmetic on the client API's clock.
//! Exactly two capabilities have no cross-platform protocol, and they are the whole reason this
//! module exists:
//!
//! 1. **Finding the game's window**, so the overlay can be placed in the game's top-left corner
//!    and follow it when the player moves it.
//! 2. **Ordering the overlay against that window**, so it is above the game but still yields to
//!    something the player deliberately raises over it.
//!
//! Neither has a portable API. On Linux the only protocol where both the game and the overlay
//! are addressable at once is X11 (see `crate::x11_overlay` for why); on Windows both come from
//! `SetWindowPos` and window enumeration.
//!
//! ## Why an enum rather than `Option`
//!
//! "I could not anchor the overlay" has two causes that need opposite responses, and collapsing
//! them into `None` would make the stacking watcher treat a Wayland-only session the same as a
//! running game that had momentarily closed. One means *stop watching, this platform cannot do
//! this*; the other means *keep watching, the game may come back*. [`AnchorOutcome`] keeps them
//! apart so the watcher's loop reads honestly.
//!
//! ## What is deliberately not abstracted
//!
//! Always-on-top, click-through, sizing and positioning are **not** wrapped here, because Tauri
//! implements all four on every supported platform and the implementations differ in ordering
//! and failure modes rather than in capability. Wrapping them would add a layer that hides the
//! ordering rules that ADR 0007 exists to record. The one exception is [`STACKING_POLL`], which
//! is a policy constant shared by the watcher on every platform.

use std::time::Duration;

use tauri::WebviewWindow;

/// A rectangle in physical screen pixels, in root/screen coordinates.
///
/// Physical rather than logical because both window APIs report physical pixels and the
/// conversion is the caller's business: the scale factor comes from the window being moved, and
/// only it knows which monitor it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
	pub x: i32,
	pub y: i32,
	pub width: u32,
	pub height: u32,
}

/// Where the game is, and whether anything else currently outranks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GamePlacement {
	/// The game window's rectangle in physical screen pixels.
	pub rect: Rect,
	/// Whether some *other* client holds an active fullscreen window.
	///
	/// Reported because it changes what the caller should claim: on Linux a focused fullscreen
	/// client sits on a layer above every keep-above window, so the overlay must stop claiming
	/// the top rather than pretend it is visible when it is not. Windows reports `false`
	/// unconditionally — see `platform::windows` for why the concept does not transfer.
	pub other_fullscreen: bool,
}

/// The result of asking the platform to anchor the overlay to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorOutcome {
	/// This platform cannot track another application's window at all.
	///
	/// Stop watching. Retrying would open a connection or enumerate the window list several
	/// times a second forever, for a capability that will never appear.
	Unsupported,
	/// The platform can do this, but no game window is running right now.
	///
	/// Keep the watcher alive: this is the ordinary state while the player is between games or
	/// looking at the app's own analysis window, and it is also what releases keep-above.
	NoGame,
	/// The overlay was anchored to the game.
	Anchored(GamePlacement),
}

/// How often the stacking watcher re-checks the game window.
///
/// 400 ms is a compromise that was measured rather than guessed: fast enough that the overlay
/// does not visibly lag behind a window being dragged, slow enough that a full window-tree walk
/// per tick is not a measurable cost on an idle desktop. The anchor request itself is skipped
/// when the game has not moved (the watcher compares against its last anchor), so the common
/// case is one geometry read per tick.
pub const STACKING_POLL: Duration = Duration::from_millis(400);

#[cfg(target_os = "linux")]
pub use linux::{find_game_window, overlay_supported};

#[cfg(windows)]
pub use windows::{find_game_window, overlay_supported};

#[cfg(not(any(target_os = "linux", windows)))]
pub use unsupported::{find_game_window, overlay_supported};

/// Platforms with neither an X server nor a Win32 window API.
///
/// macOS is the only real member: it has a window server, but the app does not implement the
/// `NSPanel` subclassing that covering another application's fullscreen Space requires, so it
/// honestly reports that it cannot do this rather than silently failing to appear.
#[cfg(not(any(target_os = "linux", windows)))]
mod unsupported {
	use super::Rect;

	/// Reports that this platform cannot find another application's window.
	///
	/// @returns Always `false`.
	pub fn overlay_supported() -> bool {
		false
	}

	/// Reports that no game window was found.
	///
	/// @returns Always `None`.
	pub fn find_game_window() -> Option<Rect> {
		None
	}
}

#[cfg(not(any(target_os = "linux", windows)))]
pub use unsupported::{find_game_window, overlay_supported};

/// Anchors the overlay directly above the game and reports where the game is.
///
/// The single entry point the stacking watcher uses, so the watcher itself carries no `cfg`
/// attributes and no platform knowledge.
///
/// @param window The overlay window.
/// @returns Whether the platform supports this, and where the overlay was placed.
#[cfg(target_os = "linux")]
pub fn anchor_overlay(window: &WebviewWindow) -> AnchorOutcome {
	let Some(id) = crate::x11_overlay::window_id(window) else {
		// A Wayland-only session has no X ids, so there is nothing to stack against. This is
		// the honest "cannot" answer, not the transient one.
		return AnchorOutcome::Unsupported;
	};
	match crate::x11_overlay::anchor_overlay(id) {
		Some(placement) => AnchorOutcome::Anchored(GamePlacement {
			rect: Rect {
				x: placement.rect.x,
				y: placement.rect.y,
				width: placement.rect.width,
				height: placement.rect.height,
			},
			other_fullscreen: placement.other_fullscreen,
		}),
		None => AnchorOutcome::NoGame,
	}
}

/// Anchors the overlay directly above the game and reports where the game is.
///
/// @param window The overlay window.
/// @returns Whether the platform supports this, and where the overlay was placed.
#[cfg(windows)]
pub fn anchor_overlay(window: &WebviewWindow) -> AnchorOutcome {
	windows::anchor_overlay(window)
}

/// Anchors the overlay directly above the game and reports where the game is.
///
/// @param window The overlay window.
/// @returns Always [`AnchorOutcome::Unsupported`] on platforms without a window API.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn anchor_overlay(_window: &WebviewWindow) -> AnchorOutcome {
	AnchorOutcome::Unsupported
}

/// Linux implementation, delegating to the X11 module.
///
/// This is a thin translation layer rather than a reimplementation: `crate::x11_overlay` holds
/// the X11 logic and its reasoning, and everything here does is map its types onto the neutral
/// ones so `lib.rs` never names an X11 type.
#[cfg(target_os = "linux")]
mod linux {
	use super::Rect;

	/// Reports whether an X server is reachable, which is what makes anchoring possible.
	///
	/// @returns `true` when XWayland or a native X session is available.
	pub fn overlay_supported() -> bool {
		crate::x11_overlay::xwayland_available()
	}

	/// Finds the game window's rectangle.
	///
	/// @returns The rectangle, or `None` when the game is not running.
	pub fn find_game_window() -> Option<Rect> {
		crate::x11_overlay::find_game_window().map(|game| Rect {
			x: game.rect.x,
			y: game.rect.y,
			width: game.rect.width,
			height: game.rect.height,
		})
	}
}

#[cfg(windows)]
mod windows;

/// Whether this platform should express "above the game" as an always-on-top claim.
///
/// The two platforms answer differently for opposite reasons, and the difference is the whole
/// reason this is a constant rather than a hardcoded `true`:
///
/// - **Windows: `false`.** `WS_EX_TOPMOST` means "above all non-topmost windows" and persists
///   even when the window is deactivated, so claiming it would put the overlay above windows the
///   player deliberately raised — exactly the behaviour the requirement rules out. Restacking
///   alone expresses the order precisely, so nothing is lost.
/// - **Linux: `true`.** A `keepAbove` window floats over other non-fullscreen windows, and the
///   stacking watcher clears the claim whenever another client goes fullscreen
///   ([`GamePlacement::other_fullscreen`]), which is the one case it cannot win. The claim is
///   what keeps the overlay visible when the *game* is not focused and no restack is being
///   applied — without it, alt-tabbing to a browser would hide the overlay entirely.
#[cfg(windows)]
pub const CLAIM_ALWAYS_ON_TOP: bool = false;

/// Whether this platform should express "above the game" as an always-on-top claim.
///
/// See the Windows definition for the reasoning; Linux keeps the claim, gated on no other client
/// holding a fullscreen window.
#[cfg(not(windows))]
pub const CLAIM_ALWAYS_ON_TOP: bool = true;

/// How far into the game window's top-left corner the overlay is inset, in logical pixels.
///
/// A visible margin rather than a flush corner: the game's own top-left corner holds the resource
/// and supply readouts, and an overlay flush against it would cover exactly the numbers a build
/// order is checked against.
pub const OVERLAY_INSET: i32 = 24;

/// Where to place the overlay, preferring the game window's top-left corner.
///
/// Anchoring to the game rather than to the screen matters because the game is not always where a
/// fixed coordinate assumes: StarCraft II opens at whatever position and size the player last
/// used. An overlay pinned to a fixed screen corner would sit off the game entirely — or, on a
/// multi-monitor desktop, on the wrong monitor.
///
/// Falls back to a fixed position when the game is not found, so the overlay still opens somewhere
/// predictable rather than not at all.
///
/// @param window The overlay window, used to read its own scale factor.
/// @param game The game's rectangle when already known, or `None` to look it up here.
/// @returns The position to move the overlay to, in logical pixels.
#[cfg(target_os = "linux")]
pub fn overlay_position(window: &WebviewWindow, game: Option<Rect>) -> (f64, f64) {
	if let Some(game) = game.or_else(find_game_window) {
		// X11 coordinates are physical pixels while Tauri positions in logical pixels, so on a
		// scaled display (the reference machine runs 3840x2160 at 200%) an unconverted offset
		// would place the overlay at twice the intended distance.
		let scale = window.scale_factor().unwrap_or(1.0);
		if scale > 0.0 {
			return (
				(f64::from(game.x + OVERLAY_INSET)) / scale,
				(f64::from(game.y + OVERLAY_INSET)) / scale,
			);
		}
	}

	(f64::from(OVERLAY_INSET), f64::from(OVERLAY_INSET) + 72.0)
}

/// Where to place the overlay, preferring the game window's top-left corner.
///
/// @param window The overlay window, used to read its own scale factor.
/// @param game The game's rectangle when already known, or `None` to look it up here.
/// @returns The position to move the overlay to, in logical pixels.
#[cfg(windows)]
pub fn overlay_position(window: &WebviewWindow, game: Option<Rect>) -> (f64, f64) {
	windows::overlay_position(window, game)
}

/// Where to place the overlay when the platform cannot find the game.
///
/// @returns A fixed top-left position, inset far enough not to sit under a title bar.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn overlay_position(_window: &WebviewWindow, _game: Option<Rect>) -> (f64, f64) {
	(f64::from(OVERLAY_INSET), f64::from(OVERLAY_INSET) + 72.0)
}

/// The window title prefixes that identify the game, lowercased.
///
/// Shared by every platform's window search so the two implementations cannot drift: a title
/// that matches on Linux and not on Windows would make the same game a supported and an
/// unsupported configuration for no reason.
///
/// `starcraft ii` is what the game's own window is called; the Wine/Proton wrapper on Linux also
/// produces `starcraft 2` spellings, and Windows builds have shipped both.
pub const GAME_WINDOW_TITLE_PREFIXES: &[&str] = &["starcraft ii", "starcraft 2"];

/// The smallest a window may be and still be considered the game.
///
/// A launcher or a splash screen can carry the game's title while being a few hundred pixels
/// across, and the real game window is never this small in any supported resolution. This is
/// what stops the overlay from anchoring to a dialog.
pub const MIN_GAME_WINDOW_SIDE: u32 = 400;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_title_prefixes_cover_both_spellings() {
		assert!(GAME_WINDOW_TITLE_PREFIXES.contains(&"starcraft ii"));
		assert!(GAME_WINDOW_TITLE_PREFIXES.contains(&"starcraft 2"));
	}

	#[test]
	fn the_minimum_side_excludes_dialogs_but_not_any_real_resolution() {
		// 800x600 was the smallest ladder resolution SC2 ever supported, so anything at or below
		// this bound must be a dialog rather than a game. Spelled `const { .. }` so this is a
		// compile-time assertion: the bounds must hold in every build, not merely in a test run.
		const { assert!(MIN_GAME_WINDOW_SIDE < 800) };
		const { assert!(MIN_GAME_WINDOW_SIDE > 320) };
	}
}

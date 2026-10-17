//! The popup that tells the player how to bring up the overlay when a game starts.
//!
//! ## Why this is a desktop notification, and why it must be *critical*
//!
//! KWin orders windows by layer, and an **active fullscreen** client sits on `ActiveLayer`,
//! above the `NotificationLayer` an ordinary desktop notification lands in (see the layer
//! table in `src/effect/globals.h` in KWin). Measured on KWin 6.7.5 with an active fullscreen
//! client: a normal-urgency notification produced **0 changed pixels on screen** — it was
//! drawn below the game and never seen — where a critical-urgency one produced 31652. So a
//! normal notification fails the one requirement the user stated: it must appear when the game
//! starts, **in windowed or fullscreen**.
//!
//! ## Why the timeout cannot be trusted, and what replaces it
//!
//! Setting a timeout on a critical notification does nothing on KDE. Plasma forces it to zero
//! in `libnotificationmanager/notification.cpp`:
//!
//! ```cpp
//! void Notification::Private::setUrgency(Notifications::Urgency urgency) {
//!     this->urgency = urgency;
//!     // Critical notifications must not time out
//!     if (urgency == Notifications::CriticalUrgency) { timeout = 0; }
//! }
//! ```
//!
//! and `setupNotificationTimeout` then refuses to start the hide timer for a zero timeout, so
//! the popup stays until the player dismisses it. Measured: still on screen at t+14s with
//! `-t 5000`, and repeated starts stacked up. A popup that never leaves would permanently
//! cover part of the game, so this module closes it itself with `CloseNotification`, which was
//! measured to remove it completely (non-backdrop pixels returned exactly to the baseline).
//!
//! ## The cost, stated plainly
//!
//! A critical notification is visually louder than an ordinary one. That is the price of it
//! being visible over a fullscreen game at all, and it is paid only at game start — once per
//! game, for [`POPUP_LIFETIME`].
//!
//! ## Windows, where only part of the above carries over
//!
//! Everything in this module's reasoning is KDE-specific, and it is worth being explicit about
//! what carries over rather than implying the same measurements exist everywhere. The facts
//! below were read out of `notify-rust` **v4.18.2** (the pinned version) rather than assumed;
//! a comment in this module previously said the opposite of the first two and had to be
//! corrected once the source was checked.
//!
//! - **`urgency` is not a no-op on Windows**: it is translated. `src/windows.rs` maps
//!   `Urgency::Critical` to WinRT's `Scenario::Reminder`, which stays on screen until dismissed
//!   (its comment says so: "stays on screen until dismissed, matching XDG spec"), while
//!   `Low`/`Normal`/`None` get the default scenario. So the same `.urgency(Critical)` call is
//!   meaningful on both platforms.
//! - **`close()` does *not* exist on Windows.** `notify-rust`'s own capability table
//!   (`src/lib.rs`) shows a blank cell for `NotificationHandle::close` under windows; the Windows
//!   `NotificationHandle` has `wait_for_action`, `wait_for_response` and `on_close` only. Calling
//!   `handle.close()` there is not a working dismissal, so the popup's lifetime on Windows is the
//!   system's to decide (and `Scenario::Reminder` asks it to keep the toast until dismissed).
//!   The detached close thread is therefore a **no-op on Windows, not a bug**, but it is worth
//!   knowing that the popup will linger there differently than on Plasma.
//! - **Whether a toast covers a fullscreen game is not established here.** Windows has its own
//!   fullscreen behaviour for toasts, and DirectX exclusive fullscreen is a different case again
//!   from borderless. This was not measurable without a Windows machine, so it is recorded as an
//!   open question rather than assumed to work.
//!
//! So on Windows the popup is best-effort in the same sense the overlay is: it is written to the
//! documented API, and the first run on real hardware is where the behaviour gets confirmed.

use std::time::Duration;

/// How long the popup stays on screen before it is closed.
///
/// Long enough to read eight or so words after a loading screen, short enough that it is gone
/// before the player is likely to be looking at that corner of the map. The player can also
/// dismiss it early: it is a real notification, so it has Plasma's own close button.
pub const POPUP_LIFETIME: Duration = Duration::from_secs(8);

/// The notification's title, which the user specified as the app name.
pub const POPUP_TITLE: &str = "SC2 Science Vessel";

/// Builds the popup's body text, naming the key the player must press.
///
/// A pure function so the exact wording can be asserted in a test: it is the whole content of
/// the feature, and a format-string mistake would ship a popup telling the player to press
/// the wrong key or nothing at all.
///
/// @param shortcut The registered overlay accelerator, e.g. `Ctrl+Alt+O`.
/// @returns The body text.
pub fn popup_body(shortcut: &str) -> String {
    format!("Press {shortcut} to bring up the overlay")
}

/// Shows the game-start popup and closes it again after [`POPUP_LIFETIME`].
///
/// Returns as soon as the popup is up; the close happens on a detached thread so the caller
/// (a Tauri command, ultimately the frontend's poll loop) is never blocked for the popup's
/// lifetime.
///
/// Failure is reported and swallowed rather than fatal: a machine with no notification daemon
/// running should still get a working overlay, just without the hint.
///
/// @param shortcut The registered overlay accelerator to name in the body.
pub fn show_game_start_popup(shortcut: &str) {
    let body = popup_body(shortcut);
    let handle = match notify_rust::Notification::new()
        .appname("sc2-science-vessel")
        .summary(POPUP_TITLE)
        .body(&body)
        // Critical is what makes it visible over a fullscreen game on KDE; see the module note.
        // It is a no-op on Windows, where the toast backend ignores urgency entirely.
        .urgency(notify_rust::Urgency::Critical)
        .show()
    {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("could not show the game-start popup ({err})");
            return;
        }
    };

    // Plasma ignores the timeout for critical notifications, so the popup is closed by hand.
    // `close()` sends a `CloseNotification` for this notification's id; it returns `()` and
    // reports nothing, so there is no failure to handle here.
    //
    // On Windows this is a no-op — `notify-rust` v4.18.2 has no `close` for its WinRT toast
    // handle (see the module note) — so the thread there is harmless rather than effective. It is
    // still spawned unconditionally so the two platforms share one code path.
    std::thread::spawn(move || {
        std::thread::sleep(POPUP_LIFETIME);
        handle.close();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_names_the_key_the_player_must_press() {
        assert_eq!(
            popup_body("Ctrl+Alt+O"),
            "Press Ctrl+Alt+O to bring up the overlay"
        );
    }

    #[test]
    fn the_title_is_the_app_name_the_user_asked_for() {
        assert_eq!(POPUP_TITLE, "SC2 Science Vessel");
    }

    #[test]
    fn a_substituted_key_is_reported_as_registered_not_as_requested() {
        // The popup must name the key that actually works. `register_overlay_shortcut` stores
        // a substitute when the requested key was taken, and telling the player to press a
        // dead combination is worse than saying nothing.
        assert_eq!(popup_body("Ctrl+Shift+F9"), "Press Ctrl+Shift+F9 to bring up the overlay");
    }
}

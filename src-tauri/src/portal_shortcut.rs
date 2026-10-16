//! Global shortcut registration through the XDG `GlobalShortcuts` portal.
//!
//! Why this exists at all: `tauri-plugin-global-shortcut` delegates to `global-hotkey`,
//! which routes **every** Linux target to `XGrabKey` with no Wayland branch. This app is a
//! native Wayland client, so that grab never reaches the compositor — it is scoped to the
//! app's own focus, which is exactly why the overlay key stopped working the moment the
//! game took focus. Injecting XTEST input in a test appeared to do nothing for the same
//! reason, and that test could therefore never have validated the shortcut (ADR 0008).
//!
//! On Wayland the correct mechanism is the compositor's own shortcut registry, reached
//! through the `org.freedesktop.portal.GlobalShortcuts` interface (portal version 2).
//! KDE implements it for real: `xdg-desktop-portal-kde`'s `GlobalShortcutsPortal` backs
//! each binding with a `KGlobalAccel` action, and `BindShortcuts` opens a "Global Shortcuts
//! Requested" dialog where the user approves or reassigns the key. Compositor-level grabs
//! are not affected by which window holds focus, which is the property the X11 path lacks.
//!
//! Two consequences are load-bearing:
//!
//! - The portal **can refuse**, and on this machine it refuses when it cannot determine an
//!   application id. `xdg-desktop-portal` derives it from the caller's systemd unit scope
//!   (`app-<id>-<random>.scope`) *and* requires a matching `<id>.desktop` file; a binary
//!   launched straight from a shell prompt has neither, so it resolves to an empty id and
//!   `CreateSession` is rejected. Registration is therefore best-effort, and the X11 plugin
//!   stays registered as a fallback for exactly that case.
//! - The user may reject the dialog. A rejected binding is still registered, deliberately,
//!   with no key assigned, so the portal does not re-prompt on every launch — that is
//!   upstream's behaviour and the reason a rejection is not retried here.

use std::future::Future;

/// Identifier sent to the portal for the one shortcut this app binds.
///
/// A stable, machine-readable id rather than the accelerator, because it is what the
/// `Activated` signal carries back and what KDE stores in `kglobalsaccelrc`. Changing it
/// would orphan the user's saved binding.
pub const OVERLAY_SHORTCUT_ID: &str = "toggle-overlay";

/// Human-readable description shown in the compositor's approval dialog.
pub const OVERLAY_SHORTCUT_DESCRIPTION: &str = "Show or hide the build order overlay";

/// Converts a Tauri-style accelerator into the XDG shortcuts trigger format.
///
/// The portal wants GTK/libxkbcommon syntax (`<Ctrl><Alt>o`), the settings file stores
/// Tauri syntax (`Ctrl+Alt+O`), and the two disagree in two ways that matter:
///
/// - Modifiers are wrapped in angle brackets, and `Alt` is spelled `Alt` in both, but a
///   trailing `+` for the plus key would otherwise terminate the modifier list.
/// - The key itself is **lowercase** for single characters. `<Ctrl>O` and `<Ctrl>o` are
///   the same physical key, but only the lowercase form is what the XDG spec calls for;
///   named keys (`F9`, `Space`, `Return`) keep their capitalisation.
///
/// Returns `None` when the accelerator does not name a pressable key — either because it is
/// modifiers only (`Ctrl+Alt`), or because it contains a token that is not a modifier this
/// function recognises. Refusing is deliberate: a silently mis-parsed accelerator would
/// bind a different key from the one the settings file promises, and the user would have no
/// way to tell the difference from the overlay simply being broken.
///
/// @param accelerator A Tauri accelerator such as `Ctrl+Alt+O`.
/// @returns The XDG trigger string, or `None` if `accelerator` names no bindable key.
pub fn to_portal_trigger(accelerator: &str) -> Option<String> {
	/// Modifier spellings this app accepts, mapped to their XDG trigger form.
	const MODIFIERS: &[(&str, &str)] = &[
		("ctrl", "<Ctrl>"),
		("control", "<Ctrl>"),
		("alt", "<Alt>"),
		("shift", "<Shift>"),
		("super", "<Super>"),
		("cmd", "<Super>"),
		("command", "<Super>"),
		("meta", "<Super>"),
	];

	let trimmed = accelerator.trim();

	// Split the trailing key off first. A trailing `+` is the plus key itself, so `Ctrl++`
	// must not be split on that final character or it yields an empty key and the literal
	// string `Ctrl` — which is what a naive split-then-pop does.
	let (modifier_text, key) = match trimmed.strip_suffix('+') {
		Some(head) => (head, "+"),
		None => match trimmed.rsplit_once('+') {
			Some((head, key)) => (head, key.trim()),
			// No separator at all: a bare key such as `F9`, with no modifiers.
			None => ("", trimmed),
		},
	};

	let mut trigger = String::new();
	for token in modifier_text.split('+').map(str::trim).filter(|t| !t.is_empty()) {
		let lower = token.to_ascii_lowercase();
		match MODIFIERS.iter().find(|(name, _)| *name == lower) {
			Some((_, trigger_form)) => trigger.push_str(trigger_form),
			None => return None,
		}
	}

	// An empty key means the accelerator was either empty or ended in a separator, and a
	// key spelled like a modifier means it was modifiers only. Neither is pressable.
	if key.is_empty()
		|| MODIFIERS
			.iter()
			.any(|(name, _)| *name == key.to_ascii_lowercase())
	{
		return None;
	}

	// Single characters are lowercased and multi-character names keep their case; `+` is
	// spelled out because a second `+` would read as another separator.
	trigger.push_str(&match key {
		"+" => "plus".to_string(),
		_ if key.chars().count() == 1 => key.to_ascii_lowercase(),
		_ => key.to_string(),
	});
	Some(trigger)
}

/// Runs an async body to completion on the calling thread.
///
/// Avoids pulling in a full async runtime: the portal work is a handful of D-Bus round
/// trips, and `ashpd` is compiled with the `async-io` feature so it shares this executor.
///
/// @param future The future to drive.
/// @returns Whatever the future resolves to.
pub fn block_on<F: Future>(future: F) -> F::Output {
	futures_lite::future::block_on(future)
}

#[cfg(test)]
mod tests {
	use super::to_portal_trigger;

	#[test]
	fn lowercases_single_character_keys_and_wraps_modifiers() {
		assert_eq!(to_portal_trigger("Ctrl+Alt+O").as_deref(), Some("<Ctrl><Alt>o"));
		assert_eq!(to_portal_trigger("Ctrl+Shift+F9").as_deref(), Some("<Ctrl><Shift>F9"));
		assert_eq!(to_portal_trigger("Alt+Space").as_deref(), Some("<Alt>Space"));
		assert_eq!(to_portal_trigger("Super+Z").as_deref(), Some("<Super>z"));
	}

	#[test]
	fn spells_out_the_plus_key() {
		assert_eq!(to_portal_trigger("Ctrl++").as_deref(), Some("<Ctrl>plus"));
	}

	#[test]
	fn accepts_a_bare_key_with_no_modifiers() {
		assert_eq!(to_portal_trigger("F9").as_deref(), Some("F9"));
	}

	#[test]
	fn lowercases_a_lone_single_character_key() {
		assert_eq!(to_portal_trigger("O").as_deref(), Some("o"));
	}

	#[test]
	fn rejects_an_accelerator_with_no_key() {
		assert_eq!(to_portal_trigger("Ctrl+Alt"), None);
		assert_eq!(to_portal_trigger(""), None);
	}

	#[test]
	fn rejects_an_unrecognised_modifier_rather_than_binding_a_different_key() {
		assert_eq!(to_portal_trigger("Hype+O"), None);
	}
}

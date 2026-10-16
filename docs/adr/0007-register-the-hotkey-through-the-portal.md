# 7. Register the overlay hotkey through the XDG portal, and show the window before applying click-through

- **Status:** Accepted
- **Date:** 2026-10-14
- **Deciders:** maintainer
- **Related:** ADR 0003 (the overlay's eventual layer-shell target), ADR 0008 (the X11 backend
  work, which proxies this ADR's key problem to the portal path)

## Context

Two independent problems surfaced from one report — "the hotkey does nothing when the game has
focus, and the overlay crashes the first time it is shown".

### The hotkey only worked while the app had focus

`tauri-plugin-global-shortcut` 2.4.1 depends on `global-hotkey` 0.8.0, whose
`src/platform_impl/mod.rs` routes **every** Linux/BSD target to `x11/mod.rs` with no Wayland
branch, and that module's only mechanism is `XGrabKey`. On Wayland the app is a native Wayland
client, so the grab is scoped to the app's own focus: pressing the key while the game held focus
delivered nothing to us. The grab was working exactly as written and was useless for the job.

KDE does implement the compositor-level alternative. `org.freedesktop.portal.GlobalShortcuts`
(interface version 2 on this machine) is routed to `org.freedesktop.impl.portal.desktop.kde`,
and the KDE backend backs each binding with a real `KGlobalAccel` action — verified by finding
`GlobalShortcutsDialog` in `/usr/lib/xdg-desktop-portal-kde` and by watching a binding reach the
portal end to end.

Three traps cost time here and are worth recording:

- **`handle_token` and `session_handle_token` are both mandatory.** Calling `CreateSession` with
  `{}` makes xdg-desktop-portal 1.22.1 **abort**: `xdp_session_initable_init: assertion failed:
  (session->token != NULL)`, systemd `status=6/ABRT`.
- **`session_handle` is typed `s`, not `o`**, contrary to the obvious reading of the spec. The
  spec acknowledges this: it was "erroneously implemented as s". A binding that demands an object
  path will fail to marshal.
- **The host `app_id` comes only from the systemd unit scope name**, and a matching `<app_id>.desktop`
  file must be installed. A dev binary launched from a shell therefore resolves to the *terminal's*
  app id (`org.kde.konsole`) — cosmetic in development, correct once packaged.

### The first show of the overlay aborted the process

`tao` 0.37.1's GTK backend unwraps the `GdkWindow` in the click-through branch only
(`src/platform_impl/linux/event_loop.rs:456`), while its four sibling call sites (430, 498, 558,
613) all use `if let Some(…)`. A window built `visible(false)` has no `GdkWindow` until it is
first shown, so `set_ignore_cursor_events` on a never-shown window unwrapped `None` and aborted
inside the GTK loop.

## Decision

**The hotkey is registered through the portal, with the X11 grab as a fallback, and the two are
mutually exclusive.** `src-tauri/src/portal_shortcut.rs` wraps `ashpd` 0.13 with the `async-io`
feature (not the default `tokio`, which would pull a second async runtime into a Tauri app that
already has one). `register_overlay_shortcut` spawns a thread, tries the portal, and falls back to
`register_overlay_shortcut_via_x11` only if the portal path returns false.

They must not both be active: on Wayland the X11 grab still fires while the app itself has focus,
so holding both would fire the handler twice per keypress and appear to do nothing. On a portal
stream error the portal path returns `true` (bound) rather than falling back, because returning
early would drop the stream and silently unbind the shortcut.

Registration happens on a background thread, not inside `setup`: `BindShortcuts` opens a
compositor approval dialog the user may take any amount of time to answer, and blocking `setup`
would mean the main window never appeared until they did.

**`to_portal_trigger` refuses rather than guesses.** It converts Tauri's `Ctrl+Alt+O` syntax to the
portal's `<Ctrl><Alt>o`, and returns `None` for an unrecognised modifier or an empty key. A
silently mis-parsed accelerator would bind a different key than the settings file promises, which
is worse than not binding at all. It splits the trailing key first, so `Ctrl++` yields the `plus`
key and not an empty one.

**In overlay mode the window is shown *before* the click-through flag is applied.** Window
requests share one ordered queue, so show-then-flag is sufficient, and the ordering is recorded
in a doc comment on `apply_window_mode` because the reverse order is a process abort rather than
a cosmetic bug.

## Consequences

- The hotkey now works with the game focused, which was the entire point of having one.
- The portal path requires an installed `.desktop` file to resolve a real app id; until packaging
  exists, a shell-launched dev build prompts under the terminal's identity. A `~/.local/share/applications/`
  entry is the dev-only workaround.
- The first press of the hotkey shows the overlay instead of killing the app.
- The X11 fallback remains necessary for Windows and macOS, which have no portal, and for Linux
  machines whose desktop does not implement `GlobalShortcuts`.
- The `wlr-layer-shell` target in ADR 0003 is unaffected: this ADR is about the key and the
  ordering, not about the surface type.

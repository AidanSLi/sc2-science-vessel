# ADR 0003 — The game overlay is a native layer-shell window on KDE, with a toplevel fallback

- Status: proposed — blocked on the Phase 0 spike (docs/assessment.md §5.2)
- Date: 2026-10-11
- Deciders: project founder (decision needed)

## Context

Measured on the reference machine (KDE Plasma 6.7.5 / kwin 6.7.5, Wayland,
`XDG_SESSION_TYPE=wayland`): the compositor socket advertises **`zwlr_layer_shell_v1`
version 5** and `zwp_keyboard_shortcuts_inhibit_manager_v1`. SC2 itself runs as a
**Wine/Xwayland** client launched through Faugus.

Known upstream limits:

- Tauri's `always-on-top` and `set_position` are unreliable on Wayland, and Tauri's
  `global-hotkey` is X11-only (tauri-apps/tao issues #1134, #14913 remain open).
- `org_kde_plasma_surface` overlays are shown above all surfaces **except fullscreen
  ones**; KDE bug 460873 tracks the missing keep-above API that yakuake-class utilities
  need.
- On macOS, covering another app's fullscreen Space requires an `NSPanel` subclass with
  `NSWindowCollectionBehaviorCanJoinAllSpaces | .fullScreenAuxiliary`
  (tauri-apps/tauri #11488) — `always_on_top` alone does not do it.

Three techniques were evaluated: (1) a native `wlr-layer-shell` surface in the overlay
layer, (2) an always-on-top toplevel over a windowed/borderless-fullscreen game,
(3) gamescope + MangoHud `custom_text` reading a file.

## Decision

Ship **(2)** as the pragmatic first implementation (documented Electron/Tauri
always-on-top + `setIgnoreCursorEvents`, which works on X11 and Wayland and is the least
code), and build **(1)** as the real KDE overlay once Phase 0 confirms it composes over
the game. **(3)** is a spike/fallback only, never the product. Hotkeys go through the
**XDG `GlobalShortcuts` portal** (implemented by KDE; xdg-desktop-portal 1.16.0+), with
the X11 grab path as a fallback since SC2 is an Xwayland client.

The overlay is rendered **outside the Tauri WebView** if it must be a layer-shell surface:
the WebView renders the analysis UI; the overlay is a Rust window.

## Consequences

- No dependence on Tauri 3 / GTK4, which has been stalled in wry#1474 since 2025-01-30
  with the maintainer's own words: "it would just really help to get out of the limbo
  we've been in for over a year now."
- If the layer-shell spike fails, the fallback degrades the product to "the player must
  run SC2 in borderless windowed mode" — a real UX constraint that must be documented for
  users rather than hidden.
- Requiring the player to grant input-device access (group `input`) is rejected in favour
  of the portal; it is a permission escalation we do not need.

# 12. The app is resident, and a game beginning is announced with a critical notification

- **Status:** Accepted
- **Date:** 2026-10-16
- **Deciders:** maintainer
- **Related:** ADR 0004 (the live clock), ADR 0010 (the mode follows the client's screen list),
  ADR 0011 (the overlay yields to a fullscreen client)

## Context

The user asked (m04856) for the app to

> "automatically start & stop when the game starts & stops",
>
> with the window hidden until the toggle key is pressed, and a hint that appears when a match
> begins so the key can be rediscovered mid-session.

Three measured facts about this desktop decided the design, and none of them were inferred:

1. **An always-on-top window cannot cover an active fullscreen client.** KWin's
   `Window::belongsToLayer()` tests `isActiveFullScreen()` before `keepAbove()`, so a focused
   fullscreen window sits on `ActiveLayer` above `AboveLayer`. This is ADR 0011, measured in
   pixels: with the red fullscreen probe active, the overlay contributed **exactly 0 pixels** to
   the screenshot; with the probe merely mapped but not active, it contributed a full rectangle.

2. **KDE ignores the timeout on a critical notification.** `notify-send -u critical -t 4000` was
   still on screen at t+8s, and repeated sends stacked. So a popup that is supposed to disappear
   cannot rely on a timeout.

3. **A window typed `_NET_WM_WINDOW_TYPE_CRITICAL_NOTIFICATION` renders above an active
   fullscreen client.** KWin's layer list is
   `…, NormalLayer(2), AboveLayer(3), NotificationLayer(4), ActiveLayer(5), PopupLayer(6),
   CriticalNotificationLayer(7), OnScreenDisplayLayer(8), OverlayLayer(9)`. Measured: with the
   red fullscreen probe active, a `CRITICAL_NOTIFICATION`-typed window was visible in the
   screenshot; a `normal` one was not.

Those three facts rule out both of the obvious designs — "open the window when a game starts"
and "an always-on-top popup" — and point at the one that works.

## Decision

**The app starts hidden and stays resident.** `Settings::start_hidden` defaults to `true`, and
`apply_window_mode` is told not to reveal the window when the mode change came from the client
API rather than from the player (ADR 0010's `Visibility::AsIs`). The window is shown only by the
toggle key, the tray menu, or the frontend's own "Show the overlay" button.

**A game beginning pops up a self-closing critical notification.** `src-tauri/src/notify.rs`
sends one notification with `Hint::Urgency(Critical)`, then closes it by id after
`POPUP_LIFETIME` (8s) on a background thread, because fact (2) means there is no timeout to lean
on. The urgency is what buys the layer that covers fullscreen, per fact (3).

**The trigger is a rise edge on the client API's `displayTime`**, polled in Rust by
`src-tauri/src/game_watch.rs` rather than in the frontend. Rust is the only option that actually
works: when the window is hidden, WebKitGTK suspends the page's timers, so a frontend poller
goes silent exactly when the app is resident — which is its whole purpose. The watcher is idle
until the client API appears at all, so a machine with the game closed costs nothing.

**A tray icon is what makes residency acceptable.** An app with no window and no dock entry is
unreachable. The tray gives the four things a player actually needs: show the overlay, show the
analysis window, open settings, and quit.

**The hint carries only the key.** `displayTime` is the only input the watcher reads, and the
notification text is fixed. Reacting to the opponent, to supply, or to anything the player is
doing would be the cheat the user ruled out (m00257); telling the player which key opens their
own predetermined build order reveals nothing about the game.

## Consequences

- The app must be running before the game is, which is why "launch at login" exists. It is
  refused for a development build: an autostart entry pointing at `target/debug/…` breaks on the
  next rebuild, and writing one silently would be worse than declining.
- A game that ends takes the client API down with it, so the score-screen transition is observed
  on a `ScreenScore` reading rather than on the API disappearing (measured: the endpoint 404s
  within seconds of the score screen).
- The popup is closed by id, not by timeout, so a player who wants it gone sooner has to wait
  the 8 seconds. Accepted — it is one small card, and the alternative (Plasma keeping it
  forever) is worse.
- The clue is deliberately low-information. A player who misses the 8 seconds gets nothing, and
  that is the correct failure mode for a hint that must not become a game-reading device.

## Still open

- **The popup has not been seen over a genuinely active fullscreen game**, only over the red
  fullscreen probe used to settle fact (3). The mechanism is measured; the real-game case is
  not.
- `tauri-plugin-notification` 2.7.0 requires Rust 1.90 (edition 2024), which raised this
  project's `rust-version` from 1.77.2.

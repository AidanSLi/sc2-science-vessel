# 16. The hotkey picks the mode it reveals, from the client's screen list

- **Status:** Accepted
- **Date:** 2026-10-18
- **Deciders:** maintainer

## Context

The app has one window with two modes (ADR 0008, ADR 0010). Until now the global hotkey was a
pure *visibility* control: `toggle_window` hid a visible window and, on a hidden one, called
`show_window`, which re-applied whatever mode the shell had last stored. The mode itself came
from two places only — the frontend's automatic `decideMode` poll, and the tray menu.

That left a hole the user named:

> make it so the overlay hotkey brings up the app window in its analysis mode when in the game
> (SC2), but not in a match.

The failure is concrete. The frontend's poll decides the mode from `/ui`'s `activeScreens`
(ADR 0010), but the app is resident and its window is **hidden between games**, and a hidden
WebKitGTK widget has its page timers suspended (`src-tauri/src/game_watch.rs` module docs, ADR
0012, `AGENTS.md`). So while the window is hidden, no poll runs and the stored mode is frozen at
whatever the last match left behind — typically `Overlay`, because the last thing the app saw was
`{"activeScreens":[]}`. A player then starts SC2, sits in the menus, presses the hotkey, and gets
a 380-pixel click-through build-order guide over the Battle.net screen instead of the analysis
window they wanted. The guide is also click-through, so it cannot even be dismissed with the
mouse.

Revealing "in the current mode" is therefore wrong exactly when it matters, and no amount of
frontend work fixes it: at the moment the key is pressed there is no live page to ask.

## Decision

**The hotkey's reveal decides its own mode, in Rust, from one Client API reading**, in
`game_watch::hotkey_mode(clock, screens) -> Option<WindowMode>`:

- `/game` unreadable → **analysis**. The client API is served by the game itself and is up for the
  whole of a match (the guide's clock depends on it), so silence means no match: no game, or the
  client API switched off. This is also what stops a stale `Overlay` coming back over the desktop.
- `isReplay: true` → **analysis**. A replay's screen list is the same empty reading as a live
  match, and the player is watching rather than playing, so its controls must be clickable.
- `/ui` unreadable → **no change** (`None`), mirroring `decideMode`'s `null`. The one way to reach
  this is a client that answered `/game` but not `/ui`, where the stored mode is the frontend's
  last decision — mid-match, the guide. Guessing *analysis* from an unreadable `/ui` would put the
  analysis pages over a running game, which is the outcome this whole change exists to prevent.
- Empty `activeScreens` → **overlay** (the guide). A match is being played.
- Any non-empty `activeScreens` → **analysis**. Menus, `ScreenLoading`, `ScreenScore` — the
  player is not playing, so a guide is useless and a clickable window is better.

The rule names **no individual screen**. Every non-empty list means the same thing for this
decision, so enumerating names would only create a second copy of `game-mode.ts`'s vocabulary to
drift out of step; a screen this app has never seen degrades to the analysis window, which is the
harmless answer. The two rules stay in step structurally: both are "empty list = playing, anything
else = not", over the same endpoint.

`toggle_window`'s show path now calls `reveal_for_hotkey`, which applies the pinned-overlay
development affordance first (so the overlay can still be inspected outside a live match) and
otherwise applies `hotkey_mode_now()`, falling back to the stored mode when the rule declines to
choose.

The choice is made **synchronously on the hotkey's thread**. Both requests go to loopback with an
800 ms budget each; deferring the reveal to a background thread would make the key feel
unreliable, which is a worse bug than 800 ms of latency on a dead socket.

## Consequences

- The user's rule now holds: hotkey in SC2's menus (or with SC2 closed) → the ordinary analysis
  window; hotkey mid-match → the guide. Hiding still hides, unchanged.
- The hotkey path makes up to two loopback requests per press, where it used to make none. Only on
  the *show* half, only when the window is hidden, and only against a local socket.
- The mode decision now exists twice, in two languages: `decideMode` (frontend, drives automatic
  switches) and `hotkey_mode` (Rust, drives the hotkey's reveal). They are deliberately separate
  because they answer different questions — `decideMode` may say "cannot tell, change nothing",
  whereas the hotkey must still show *something* when there is no game at all. The tests on both
  sides pin the same captured payloads, so a divergence shows up as a failing test rather than as
  a guide over the menus.
- `show_window` keeps its old meaning ("show in the stored mode") for the watcher's
  show-my-window request, which is a player clicking the app icon and wanting back the window they
  left. Only the hotkey re-decides.
- A future change to the screen list's *meaning* has to be made in `src/lib/game-mode.ts` **and**
  `src-tauri/src/game_watch.rs`. That is the cost of the split, accepted because the alternative —
  a Rust process waiting on a suspended webview — cannot work at all.
- If `/ui` ever starts answering during a match with a non-empty list (for example an in-game
  panel this app has not seen), the hotkey lands on the analysis window mid-match. Recoverable by
  pressing the key again, and the guide is still what the automatic switch would have set; the
  worse failure — the analysis pages over a live game — is not reachable from an unreadable
  endpoint.

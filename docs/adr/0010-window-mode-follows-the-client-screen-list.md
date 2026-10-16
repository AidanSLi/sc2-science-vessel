# 10. The window switches modes on the client's screen list, not on a clock stall

- **Status:** Accepted
- **Date:** 2026-10-16
- **Deciders:** maintainer

## Context

The app is **one window with two modes** (ADR 0008's geometry work, and the user's own rule:
"There is no separate 'app window' and 'overlay window'. It is all the app window"). It shows the
build-order guide over the game, and the ordinary analysis view otherwise. Swapping between them
has to be automatic: the user asked that "as soon as the match is complete and the 'score screen'
button is pressed, the app should transition from 'build order overlay' mode to the standard
mode", without pressing anything.

The first implementation inferred "the game has ended" from **the clock standing still**, on the
theory that `displayTime` freezing means either a pause or a finish. It picked a five-second
threshold and had to choose which error to prefer.

That theory was wrong in a way that could not be fixed by tuning the threshold. The clock freezes
on a pause **and** at the end of a game, so the two are genuinely indistinguishable from the
clock alone — which means any threshold either threw the player out of the build guide during a
pause (the worse error: the guide is what they are looking at while playing) or left the guide up
after the game. There was no value that avoided both.

The client had the answer all along, and it had been dismissed: `/ui` was polled once during a
game, returned `{"activeScreens":[]}`, and was written off as carrying no information. **An empty
list was the information.** It means "no UI screen is open", which is exactly the in-game state.

## Decision

**`decideMode` is a pure function of one client reading, and it switches on `/ui`'s screen list.**

- `{"activeScreens":[]}` → the build-order guide. The in-game reading.
- `ScreenScore/ScreenScore` present → the analysis view. The post-game screen, i.e. exactly the
  transition the user asked for by name.
- Any other `Screen*` name → the analysis view. Menus, including `ScreenLoading` on the way into
  a game.
- `/ui` unreadable (`screens === null`) → **no change**. `null` is deliberately distinct from an
  empty list; collapsing them would put a build-order guide over the player's menus on a
  transient HTTP failure.
- A replay → the analysis view, checked **before** the screen list because a replay's list is the
  same empty reading as a live game, and a replay is being watched rather than played, so its
  controls must be clickable.

Screen names were recovered from two working consumers of the endpoint
(`leigholiver/OBS-SC2Switcher`'s `menuLabels`, and `Rairden/sc2-scenes`, which ships captured
request/response transitions) because **there is no official documentation** — the
s2client-proto `docs/` folder contains only `linux.md`, `protocol.md` and a PNG. `/game` and
`/ui` are read together and answer different questions: `/ui` says where the player *is*, `/game`
says what the game is *doing*.

**`decideMode` returns a target mode, never a delta, and carries no elapsed-time state.** The
"has it changed?" comparison lives in the caller, against `#lib/window-mode`'s store, because that
store mirrors the shell and is the only authority on what the window actually is. A changed-flag
inside `decideMode` would be a second copy of that state and would drift the first time a mode was
set from anywhere else — the settings page, or the shell's own startup affordance.

**Mode and visibility stay independent** (see ADR 0008's `Visibility` argument): an automatic mode
change leaves the window exactly as visible as the player left it, and never re-centres or steals
focus.

## Consequences

- A pause no longer changes the view, which is the whole reason for the change.
- The end-of-game switch is exact rather than a guess, and it happens on the score screen — the
  moment the user named.
- The app now depends on an **undocumented** endpoint. That is a knowing risk: the behaviour was
  verified against captured traces and must be re-verified against a real game (see
  `docs/salt-encoding.md`'s sibling practice of verifying claims by decoding real payloads rather
  than by reading about them). A change to the screen names degrades to "the mode stops changing",
  which is recoverable with the hotkey, not to a wrong mode.
- `/ui` is polled alongside `/game`; a `/ui` failure no longer fails the poll, because the clock
  is the more important of the two.
- The five-second `CLOCK_STALL_SECONDS` constant and its associated tests were deleted, along with
  the pause/end ambiguity they encoded.

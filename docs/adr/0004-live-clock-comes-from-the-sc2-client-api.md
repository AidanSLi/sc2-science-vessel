# ADR 0004 — The live build-order clock reads the SC2 Client API, never a wall-clock timer

- Status: proposed — **must be validated by the Phase 0 spike before anything is built on it**
- Date: 2026-10-11
- Deciders: project founder

## Context

The requested behaviour is that build steps "follow in-game time" and the timer "pauses
when the game is paused". A replay cannot do this: it stores **game time**, and a
wall-clock timer keeps running through a pause and drifts with any speed change.

The one documented live channel is Blizzard's SC2 Client API on `localhost:6119`, which
the client exposes by default. From Blizzard's 2016 developer post:

> "The Client API is accessed via HTTP GET queries and returns JSON responses. By default
> the SC2 client will listen on port 6119 for localhost queries only."
> "The `displayTime` field is in seconds; it reflects the timer in the UI."

`GET http://localhost:6119/game` →
`{"isReplay":false,"displayTime":100.0,"players":[…]}` with player name/type/race/result.
That is **clock and identity only** — no units, no supply, no build state.

Rejected alternatives: reading `/dev/input` for hotkeys (permission escalation), reading
game memory (EULA risk, fragile — see Scelight's registry-based Live APM, icza/scelight
issue #7), Overwolf GEP (SC2 support is deprecated; its only features are `gep_internal`
and `match_info` with `match_start`/`match_end` — no clock), and assuming the replay file
is appended during a live game (**UNVERIFIED — no source found**).

## Decision

The overlay's clock is `displayTime` polled from `http://localhost:6119/game` at ~1 Hz.
Matchup/build preset selection uses the same response. "A game just started" is detected
from `isReplay: false` plus an advancing `displayTime`. **No wall-clock timer is ever used
to advance a build order.** If the endpoint is unavailable, the feature degrades to a
manually advanced (hotkey/spacebar) build list and says so in the UI, rather than
pretending to track a game it cannot see.

## Consequences

- Pause, speed changes, and replay-mode are handled by the game itself — which is exactly
  the requested behaviour, and for free.
- The richest requested feature, "prompt me about what to expect next based on what the
  opponent is doing", is **not achievable** through this channel and is out of scope for
  the overlay. Anything state-aware would require screen-reading/OCR and carries a
  materially higher EULA profile. The app will only offer clock-indexed expectations.
- The whole overlay milestone is gated on a 15-minute experiment: run SC2, curl the
  endpoint, confirm `displayTime` ticks and pauses. This is the first task of the project.

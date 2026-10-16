# 9. Centralise event ownership in `sc2sv.identity`

- **Status:** Accepted
- **Date:** 2026-10-14
- **Deciders:** maintainer

## Context

Attribution — deciding which player a replay event belongs to — is the most error-prone part of
this codebase, because `sc2reader` exposes ownership through **four different fields with three
different conventions**, and three of the four fail in a way that still produces plausible
output rather than raising.

Verified against the reference library (`sc2reader` 1.9.0):

1. **`event.control_pid`** — on `UnitBornEvent` and `UnitInitEvent`. A real player id, and `0`
   for neutral units. The only usable field on births.
2. **`event.player`** — a `Participant` on game events and on `PlayerStatsEvent`. Reliable
   there, and **`None` on `UnitBornEvent`, `UnitInitEvent` and `UnitDiedEvent`**. Reading
   births through `event.player` yields nothing at all — which is exactly what happened: every
   unit count came back empty.
3. **`event.pid`** — a **0-based index** into `replay.players` on game events, but a **real
   player id** on `PlayerStatsEvent`. Two meanings for one name. A truthy or set-based test on
   it also drops index `0`, i.e. the first player.
4. **`event.unit.owner`** — a `Participant` on the `Unit` attached to a `UnitDiedEvent`, and
   `None` for neutral. This is the **only** way to attribute a death: `UnitDiedEvent` carries no
   `control_pid` and no `player`, only `unit_id`. Reading a death's owner from `unit_owner_pid`
   finds nothing, because no such attribute exists.

`team_id` compounds the problem: the first team holds `team_id == 0`, so a truthiness check
silently discards a whole side. Teams must be compared with `==`, never tested for truth.

The measured evidence that this is a real trap rather than a theoretical one: on the reference
library `event.player is replay.players[event.pid]` holds for **100%** of events, while
`event.pid == event.player.pid` holds for **0%** of them — so using `event.pid` as an id
mislabels every game by one player, and it looks correct because player 1's events still land on
a player 1.

## Decision

**All attribution goes through `sidecar/sc2sv/identity.py`.** The module documents the four
conventions in its docstring, exports `PlayerDirectory` as the single lookup, and owns
`NEUTRAL_CONTROL_PID = 0`. `_OWNERSHIP_FIELDS` lists the fields in the order they are
consulted, each tagged with whether its value is a real player id or an index, so the fallback
order is data rather than a chain of conditionals.

No metric module reads an ownership field directly. `tests/test_identity.py` pins the four
conventions, and each assertion is written so that reading the *wrong* field produces a
specific, non-empty but **incorrect** answer — a test that failed loudly on a `None` would not
have caught the original bug, because the original bug returned `None` gracefully.

## Consequences

- The composition counts are correct, and the four conventions are stated in one place instead
  of being rediscovered per metric.
- `tests/test_identity.py` is the regression suite for the composition bug and must be kept
  passing when a new event type is read.
- A new metric that needs an ownership field has to go through `PlayerDirectory`; adding a
  direct `event.player` read elsewhere re-opens the hole this ADR closes.
- Every documented claim here is checkable against a fixture replay, so the docstring is
  evidence rather than folklore.

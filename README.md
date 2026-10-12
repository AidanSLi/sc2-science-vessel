# sc2-science-vessel

An offline, account-free desktop app for StarCraft II replay analysis — the
SC2ReplayStats-style metrics you actually use to improve — plus a build-order
reference/coach surface modelled on SpawningTool's build library and Overwolf overlay.

Two halves, deliberately separable:

1. **Analysis** — scan a local replay library, parse it, and show worker/resource/
   supply/build-order/tech/battle metrics. Needs no network and no Blizzard account.
2. **Live coach** — an always-on-top overlay that follows the in-game clock through a
   build order, keyed to the matchup, and advances/pauses with the game.

**Read [`docs/assessment.md`](docs/assessment.md) first.** It is the feasibility study
this project was founded on: every claim in it was checked against real replay data and
the real desktop session on this machine, and it is explicit about which claims are
**UNVERIFIED**. The two decisions that shape the whole build are in its §5.

## Status

Founding stage. No application code yet — the assessment, the ADRs, and the project
conventions are written; the first milestone is the Phase 0 de-risk experiment described
in `docs/assessment.md` §5.2 (confirm the SC2 Client API clock on `localhost:6119`).

## Layout

```
docs/assessment.md   feasibility study — read this first
docs/adr/            architecture decision records
AGENTS.md            conventions for agents working in this repo
```

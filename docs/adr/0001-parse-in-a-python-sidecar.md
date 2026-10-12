# ADR 0001 — Replay parsing runs in a Python sidecar, not in Rust or the WebView

- Status: accepted
- Date: 2026-10-11
- Deciders: project founder (agent-proposed, to be confirmed)

## Context

The app must turn a `.SC2Replay` archive into metrics. Two libraries do this today, both
verified on this machine against the user's own 722-replay library:

- **`sc2reader 1.9.0`** (Python; `requires_python >=3.9`, classifiers up to 3.14,
  released 2026-06-25, upstream pushed 2026-10-05) — decodes `replay.tracker.events`
  (`PlayerStatsEvent`, `UnitBornEvent`, `UnitDiedEvent`, `UpgradeCompleteEvent`,
  `UnitInitEvent`, …) and every header field we need.
- **`spawningtool 3.0.0`** (Python, **MIT**, Copyright (c) 2013 Kevin Leung) — turns the
  same replay into a supply-ordered build order (69 steps extracted from a real ZvT).
  It ships ~1,900 lines of LotV unit/build-time tables in `lotv_constants.py`.

The alternatives:

- **`s2protocol-rs`** (Rust, sebosp, 118 stars, pushed 2026-10-08) decodes the protocol
  but does not give metrics or build orders. Choosing it means reimplementing
  spawningtool's data tables by hand.
- **TS/JS in the WebView** — no maintained replay parser exists, and the analysis
  payload (tens of thousands of points per game) must not cross the WebView boundary raw.

## Decision

Parsing and metric computation live in a **Python sidecar** bundled with the Tauri app
(PyInstaller, wired through Tauri's `externalBin`). The sidecar's public surface is
**JSON in / JSON out** — it returns *prepared series* (downsampled, bucketed, already
aggregated), never raw event lists. The frontend never parses a replay.

## Consequences

- We inherit MIT-licensed build extraction and an actively maintained decoder instead of
  writing ~1,900 lines of game data.
- The sidecar is a real process boundary: it must be versioned, bundled per platform, and
  started/health-checked by the Rust shell. That is the price.
- Because the boundary is JSON-only, the metric layer can be developed, tested, and even
  shipped as a pure CLI before any UI exists. Phase 1 of the roadmap depends on this.
- If the sidecar ever needs to be replaced, the contract (`series` JSON) is the only thing
  the frontend knows about, so the replacement is bounded.

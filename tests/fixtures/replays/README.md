# Test replay fixtures

Real `.SC2Replay` files used by the metric tests. Metrics are never tested against
synthetic data alone — the failure mode that matters is "this field is named differently
in this build version".

## Required fixtures

| File | Why |
| --- | --- |
| `zvt_build_95841.SC2Replay` | the one replay on the reference machine (`22 drone ling bane`, build 95841, ZvT vs AI). Small; exercises build-order extraction and worker/army graphs. |
| `zvp_build_97563.SC2Replay` | newest build in the local library (97563). ZvP. Exercises modern tracker data. |
| `tvp_build_93333.SC2Replay` | oldest build in the library (93333, 2025-01-29). Guards against regressions on old builds. |
| `paused_game.SC2Replay` | a game known to contain a pause, to test that the timeline stays on the game clock. |

Replays are **not** committed here (they are large and user-specific — the ignore rules
cover `*.SC2Replay`). Copy them in from the local library before running the fixture
tests; the tests skip with a clear message when a fixture is absent.

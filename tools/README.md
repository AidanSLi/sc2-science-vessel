# tools/

Development-time scripts. Nothing here ships inside the app, and nothing here is needed
for the app to run offline.

- `refresh_build_snapshot.py` (planned) — scrapes SpawningTool's public build pages into
  the bundled `builds.json` corpus. This is the **only** component allowed to touch the
  network, it runs at development/build time, and it must degrade to "keep the existing
  snapshot" on any failure. See `docs/adr/0002`.
- `scan_library.py` (planned) — parses a whole replay directory at header level and prints
  a table (build version, matchup, length, MMR presence). The feasibility study used this
  against 722 replays at ~3 ms each.
- `extract_build_order.py` (planned) — prints the extracted build order for one replay via
  `spawningtool.parser.parse_replay`.

## Reproducing the feasibility numbers

```bash
uv venv .venv
uv pip install --python .venv/bin/python sc2reader==1.9.0 spawningtool==3.0.0
.venv/bin/python tools/scan_library.py "<replay dir>"
```

The clock the overlay must trust, while a game is running:

```bash
curl -s http://localhost:6119/game
```

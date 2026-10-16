# tools/

Development-time scripts. Nothing here ships inside the app, and nothing here is needed
for the app to run offline.

- `refresh_build_snapshot.py` (planned) — scrapes SpawningTool's public build pages into
  the bundled `builds.json` corpus. This is the **only** component allowed to touch the
  network, it runs at development/build time, and it must degrade to "keep the existing
  snapshot" on any failure. See `docs/adr/0002`.
- `scan_library.py` — parses a whole replay directory at details level and prints a table
  (build version, matchup, length, MMR presence). The feasibility study used this against
  722 replays at ~3 ms each. It is standalone on purpose: it imports `sc2reader`/`mpyq`
  directly instead of `sc2sv`, so it can still run when `sc2sv` will not import.
- `overlay_fullscreen_rule.sh` — `install`/`remove`/`status` for the opt-in KWin window rule
  that lifts the app's window into KWin's `OverlayLayer`, which is the only band above an
  *active fullscreen* window. Needed because a fullscreen client defeats `always_on_top` on
  KWin by design (`Window::belongsToLayer()` tests `isActiveFullScreen()` before
  `keepAbove()`), and stacking order sorts purely by layer index. It is a desktop-wide change
  — `OverlayLayer` is above every fullscreen window, including other applications' — so it is
  user-run and never applied by the app. The class string in it must be **capitalised**
  (`Sc2-science-vessel`): KWin matches `WM_CLASS` case-sensitively, and a lowercase rule matches
  the resource *name* and silently never fires. See `docs/adr/0011`.
- `extract_build_order.py` (planned) — prints the extracted build order for one replay via
  `spawningtool.parser.parse_replay`. Note that its `time` labels use the pre-LotV 22.4 fps
  constant; use `frame / 16`, or `sc2sv.builds`, which already does.

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

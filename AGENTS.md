# sc2-science-vessel — conventions

## Language and toolchain

- **Python 3.11+** for the analysis sidecar. It depends on `sc2reader` (1.9.0 tested) and
  `spawningtool` (3.0.0 tested); both are actively maintained and `spawningtool` is MIT.
- **TypeScript + SvelteKit + Tailwind** for the UI.
- **Tauri 2** for the shell. Do **not** design anything that requires Tauri 3 / GTK4:
  the GTK4 migration (wry#1474) has been stalled since 2025-01-30 with no landing date.
- Package manager: **`uv`** for Python (`uv venv`, `uv pip install`); **`pnpm`** for JS
  (12.11.2 here). Use pnpm, never npm — the global preference is recorded in
  `~/.pi/agent/AGENTS.md`, and `package.json` pins `packageManager`.

## Style

- Use **tab characters** for indentation in generated code (Python, TS, Rust, JSON, YAML).
  Never emit space-indented blocks.
- Functions longer than 10 lines must carry a docstring / JSDoc comment / rustdoc comment
  saying what it does and why, not just what the parameters are.
- Docstrings and comments explain **why**, not what. A comment that restates the code is
  deleted. A comment that records a measured constant, an upstream bug, or a rejected
  alternative is kept.

## Linux runtime notes

- **The main window starts hidden and is shown by the frontend, with a watchdog behind it.**
  `setup` arms a 10s thread and the root layout invokes `frontend_ready` from `onMount`; both
  call `show_main_window`, which is idempotent. The failure mode is "the window is late",
  never "the window never appears" — keep it that way. Do not reorder the watchdog to run
  *after* awaiting the frontend signal, and do not replace `isTauri()` with a
  `window.__TAURI__` check: that global is only injected when `withGlobalTauri` is enabled,
  and this app does not enable it. See `docs/adr/0006-reveal-the-window-on-frontend-paint.md`.

- **WebKitGTK's DMABUF renderer is broken on some Mesa/driver combinations, and the failure
  is silent.** Observed two ways on the reference machine (KDE 6.7 Wayland, webkit2gtk
  2.54.1, NVIDIA RTX 4070 + AMD iGPU): the process dies with `Error 71 (Protocol error)
  dispatching to Wayland display`, or an X11 window opens blank while `Failed to create GBM
  buffer of size 1280x820: Invalid argument` repeats on stderr. The app fixes this itself in
  `src-tauri/src/main.rs` — do not add the env var to commands, and do not remove that block
  without the evidence in `docs/adr/0005-disable-webkitgtk-dmabuf-renderer.md`.
- **Process cleanup is a trap in three separate ways.** `pkill -x sc2-science-vessel` does
  nothing, because Linux truncates `comm` to 15 characters (`sc2-science-ves`). A `pkill -f`
  pattern also matches the invoking shell's own command line, so an inline cleanup command
  kills the harness that is running it. And `ps | grep -c <pattern>` counts the `grep` itself,
  so a "0 processes left" check always reads 1. Put cleanup in a script file, count by `comm`,
  and match by port for anything serving 1420 — a stale Vite makes the next `tauri dev` fail
  outright, while a stale window looks exactly like a fresh one.
- **Global shortcuts are X11-only** (Tauri issue 3578). Registering on Wayland is expected to
  fail, which is why `setup` logs the error and continues instead of returning it: an error
  out of `setup` takes the whole app down, leaving no window to toggle the overlay from.

## Testing

- Any metric implementation lands with a fixture test against a real replay from
  `tests/fixtures/replays/`. Do not unit-test metrics against synthetic data only — the
  failure mode that matters is "the field name is different in this build".
- The test suite must run with **no network**.
- Fixture replays are **not** committed (see `tests/fixtures/replays/README.md`). Tests that
  need one must `pytest.skip` with a message naming the file to copy in, so a fresh clone
  still goes green. `tests/conftest.py` owns resolving that path.
- Parsing a real replay takes 2-3 ms at header level but **0.3-1 s once events are decoded**
  (`load_level=4`, tens of thousands of events). Keep fixture tests to the handful of replays
  that must actually be parsed, and load a replay once per test module where possible.

## The game clock (do not get this wrong again)

This was initially recorded backwards and cost real effort to undo, so it is stated plainly:

- **The SC2 game clock is 1:1 with real time.** Legacy of the Void removed the Wings of
  Liberty / Heart of the Swarm "Blizzard time" offset. At `Faster` there is no multiplier.
  A value on the game clock equals the same number of real seconds of unpaused play.
- **`frames / 16` is that clock.** It is the only correct way to turn a replay's frames into
  seconds.
- **`sc2reader`'s `replay.length` / `real_length` are wrong for LotV.** `resources.py` divides
  by `16 * 1.4` for every build >= 34784 regardless of expansion, while the LotV `Faster`
  factor in `constants.py` is `1.0`. The numbers are small in absolute terms but they are
  systematically ~1.4x low, which is exactly the kind of error that looks plausible.
- **Never add a speed multiplier.** `gameToRealSeconds` was removed rather than fixed: a
  helper that accepts a multiplier implies the multiplier is sometimes not 1, which is how
  the wrong factor spread through the docs in the first place. If a real-time figure is
  needed, it equals the game-clock figure.
- **The clock stops during a pause.** Game seconds are therefore not elapsed real time. Where
  that distinction matters ("how long since the last worker"), say which one is meant.

Evidence on this machine: the live client API's `displayTime` advanced 309 s over 304 s of
wall clock; replay `Duration` matched `frames / 16` within 1% across 722 replays. Re-measured
across the whole 783-replay library: the median of `Duration / (frames/16)` is 1.0000 over 708
LotV replays, and `Duration / (frames/22.4)` is 1.4001, so `frames/16` is the displayed clock
and there is no extra multiplier to apply.

## Hard rules learned the hard way

- **`load_level=4` is required to get any events at all.** `load_level=2` (details) returns
  `game_events == []` and `tracker_events == []`, and `load_level=0` additionally returns no
  players, no map, no speed and no game type. Anything that reads events must load level 4;
  anything that only reads headers should still use 2, because 0 is silently empty rather
  than raising. Cost: about 2-3 ms per replay either way.
- **`event.player` is authoritative; `event.pid` is a 0-based index into `replay.players`,
  not `Player.pid`.** On the reference library `event.player is replay.players[event.pid]`
  holds for 100% of events, while `event.pid == event.player.pid` holds for 0% of them. Real
  IDs start at 1, so using `event.pid` as an ID mislabels every game by one player — and it
  looks plausible because player 1's events still land on a player 1.
- **The energy abilities have non-obvious names**, and they are the only way to build the
  inject/MULE/chrono features: Zerg inject is `SpawnLarva`, Terran MULE is `CalldownMULE`,
  Protoss chrono boost is `ChronoBoostEnergyCost`. There is no `Inject` or `MULE` ability
  name. All three carry `event.player` and a `second`, and the Zerg/Terran ones carry a
  `location`, so a hatch/CC can be identified from the cast position.
- **Read an ability's name from `event.ability_name`, never `event.ability.name`.** A smart
  cast (a queen right-clicking a hatchery, which is how inject is *normally* cast) resolves to
  `ability.name == 'RightClick'` while `ability_name` keeps `SpawnLarva`. On one replay 198 of
  237 injects read as `RightClick` and only 39 were found, so 84% of injects were invisible and
  the reported idle time was understated — the player looked better than they were. This is the
  dangerous kind of bug: it returns a plausible number instead of failing.
- **Measure energy downtime per player, against that player's own town halls, and only then
  merge.** A mirror matchup has two players fielding the same ability, and pooling both players'
  casts against the union of their bases lets a cast between two players' hatcheries be credited
  to the wrong one. Measured on a real 1v1 ZvZ (`White Rabbit LE (24)`): per-player attribution
  gives 2578.7 s where pooling gave 2116.5 s, an 18% under-report, and the pooled base count
  (8) conflated two players. The payload's `energy.<kind>.perPlayer` carries the breakdown so the
  UI can name whose macro was idle; `merge_energy_timelines` merges without re-summing per second.
- **`cache.CACHE_REVISION` must be bumped for any metric change.** A cache entry is keyed by
  path, size, mtime *and* the revision, and a stale entry is indistinguishable from a fresh one:
  a before/after comparison silently re-read the old numbers once and appeared to prove a fix had
  not worked. When in doubt, clear `~/.cache/sc2-science-vessel/` before re-measuring.
- **`PlayerStatsEvent` is sampled every 10 game seconds** (observed cadences `[0, 10]`,
  `[0, 5, 10]`, `[0, 3, 10]`). Never claim sub-10-second precision for a stat derived from
  it; use per-unit tracker events when you need more.
- **`UnitDiedEvent.unit_type_name` is `None`.** Join deaths to births on `unit_id` to
  recover the unit type. Do not trust the field.
- **Filter cosmetic noise from `UpgradeCompleteEvent`** (`Spray*`, `Reward*`, `Dance*`),
  and neutral/map spawns from `UnitBornEvent` (`MineralField*`, `VespeneGeyser`,
  `InvisibleTargetDummy`, `XelNagaTower`). Both pollute counts badly.
- **`replay.gamemetadata.json` is a member inside the MPQ archive, not a file beside the
  replay.** (MMR, `Duration`, map, per-player results live there.) Read it through the
  archive; `Path(...).with_suffix(".SC2Replay.gamemetadata.json")` silently finds nothing.
  Its `Players` is a **list**, not a dict keyed by player id, and only ranked games carry an
  `MMR` at all (251 of 783 here). One replay in the library carries `MMR: -36400` (an arcade
  map), so validate before using. `Duration` is in **seconds**, 1:1 with real time, and
  matches `frames / 16` — so it is safe to use, unlike `sc2reader`'s `replay.length`.
- **`spawningtool`'s step `time` strings are on the wrong clock; use `frame` instead.**
  `spawningtool/lotv_constants.py` hardcodes `FRAMES_PER_SECOND = 22.4` — which is
  `16 * 1.4`, the pre-LotV Faster factor — and formats its `M:SS` labels with that divisor.
  The `frame` values are raw engine frames and are correct, so the fix is
  `second = frame / 16`. `sidecar/sc2sv/builds.py` owns this conversion and is the only
  place that should touch `spawningtool`'s step shape. Verified by joining the two streams:
  `spawningtool`'s pylon frames read `[44.44, 168.69, 234.00, 307.75, …]` at `/16` while the
  tracker stream's `UnitInitEvent` seconds are `[44, 168, 234, 307, …]`, and the median
  disagreement across 826 matched steps is 4.3 s, versus tens of seconds at `/22.4`. This is
  why it matters: the build-order comparison has a **+/- 10 second** tolerance, so a 1.4x
  clock error would put every step 40% off and report every build as mistimed, with no error
  anywhere to point at.
- **`spawningtool`'s `parse_replay` result has no top-level `build` step list.** Top-level
  `build` is the build **number** (`93333`). Steps live at `players[pid]["buildOrder"]`,
  keyed by the **string** `"1"`/`"2"` rather than by int pid, and each step is
  `{frame, time, name, supply, is_worker, clock_position, is_chronoboosted}`. Ability and
  unit-loss entries have no `supply` key at all, so read it defensively.
- **There is ONE window, and it has two *modes* — not two windows.** The user's rule: "There
  is no separate 'app window' and 'overlay window'. It is all the app window." The mode is
  either `analysis` (resizable, decorated, interactive — matches, builds, analysis, settings)
  or `overlay` (borderless, transparent, click-through, pinned over the game). The window's
  own view is swapped by `src/routes/+layout.svelte` rendering either `<BuildOrderGuide />` or
  the route children. Never create a second window; a second one duplicates the route and the
  flags and the two drift apart. `apply_window_mode` in `src-tauri/src/lib.rs` is the **only**
  thing that may set decorations, click-through, always-on-top, `skip_taskbar` or the size,
  because those flags do not reliably survive a hide/show cycle: a click-through overlay that
  has quietly become clickable is discovered mid-game, which is the worst time.
- **Mode and visibility are independent, and conflating them is a bug.** `apply_window_mode`
  takes a `Visibility` (`Shown` for the hotkey, `AsIs` for an automatic change). Without this,
  hiding the app mid-game and then finishing the game pops the window back up uninvited over a
  player who asked for it to be gone. Re-centring and `set_focus` are likewise confined to
  `Visibility::Shown`: an automatic scan of client state must never move the window the player
  placed or pull focus out of a running game. The rule is `should_reveal()`, with tests.
- **Mode changes come from the client's screen list, never from the game clock.** The clock
  freezes both while paused *and* when the game ends, so it cannot tell the two apart — an
  earlier version used a five-second stall threshold and would throw a player out of the build
  guide on a pause. Read `/ui` (see the next rule).
- **`/ui`'s `activeScreens` is the game-state signal — an EMPTY list is the in-game reading.**
  It was dismissed early on because one probe during a game returned `{"activeScreens":[]}` and
  that was read as "no information". Empty *means* "no UI screen is open", i.e. a game is being
  played. `ScreenScore/ScreenScore` is the post-game screen (the transition the user asked for
  by name — "as soon as the match is complete and the 'score screen' button is pressed"), other
  `Screen*` names mean menus, and `ScreenLoading` means a map is loading. Names come from two
  working consumers of the endpoint (`leigholiver/OBS-SC2Switcher`, `Rairden/sc2-scenes`, the
  latter with captured transitions) because **there is no official documentation** — the
  s2client-proto `docs/` folder holds only `linux.md`, `protocol.md` and a PNG. `/game` and
  `/ui` are read together and answer different questions: `/ui` says where the player *is*,
  `/game` says what the game is *doing*.
- **`screens: null` (could not read `/ui`) is NOT the same as `screens: []` (in a game).** In
  `src/lib/client-api.ts` `fetchActiveScreens` returns `null` on failure; `decideMode` returns
  `null` for it and the view is left alone. Collapsing the two would put a build-order guide
  over the player's menus on a transient HTTP failure.
- **`decideMode` is a pure function of one reading — no elapsed-time state.** It returns the
  target mode and never a delta; deciding not to repeat an already-applied mode is the caller's
  job, because `#lib/window-mode`'s store mirrors the shell and is the only authority on what
  the window actually is. A "changed?" flag inside `decideMode` would be a second copy of that
  state and would drift the first time a mode were set from anywhere else.
- **A minimum window size is a hard floor the WM enforces, so it must move with the size.**
  Leaving the 900x600 analysis floor in place while sizing down to the 380x300 overlay clamps
  the overlay straight back to 900 wide. `apply_window_mode` therefore sets `min_size`/`max_size`
  **per mode**; `tauri.conf.json` declares the same numbers for the first paint. **Clearing the
  constraints to `None` is NOT enough** — the window's `minWidth`/`minHeight` from
  `tauri.conf.json` are re-asserted by the toolkit, so the overlay still came up 1024x656
  (measured) instead of 380x300. The overlay branch now pins `min_size` **and** `max_size` to
  exactly `OVERLAY_WIDTH`/`OVERLAY_HEIGHT`, making the size a fact rather than a request that
  something else can refuse. Verify a window-size change by reading the real geometry back
  (`qdbus6 org.kde.KWin /KWin org.kde.KWin.getWindowInfo <uuid>`) — a `set_size` that returns
  `Ok` proves nothing about the size the window ended up with.
- **A fullscreen client defeats `always_on_top`, so the overlay yields instead of pretending.**
  KWin's `Window::belongsToLayer()` tests `isActiveFullScreen()` **before** `keepAbove()`, so a
  *focused* fullscreen window sits on `ActiveLayer` — above anything a keep-above toplevel can
  reach — and `Workspace::constrainedStackingOrder()` sorts purely by layer index, so no
  restacking crosses layers either (ADR 0008's mechanism cannot help here). The watcher
  therefore reads `_NET_WM_STATE_FULLSCREEN` off the same X connection it restacks over and
  **drops keep-above while another client is fullscreen** (`desired_keep_above`, tested), so the
  app reports the state it actually has and the overlay stays recoverable (raise it) rather
  than invisible-but-present over a game that is drawing on top of it. Layers that *are* above
  `ActiveLayer` (`OverlayLayer`, reachable via a KWin rule or `wlr-layer-shell`, ADR 0003/0011)
  also cover other applications' fullscreen windows, so neither is the default. Note that
  `isActiveFullScreen` is focus-conditional: a fullscreen game that lost focus is *not* on
  `ActiveLayer`, which is why a windowed or unfocused game keeps working normally.
- **The global shortcut cannot be tested by injecting keys on this machine.** XTEST
  synthesises keys but they do **not** reach XGrabKey grabs through Xwayland: a bare
  `XGrabKey` + XTEST test showed the grab never firing, so a hotkey test that "did nothing"
  proves nothing about the app. To test the *registration* path instead, hold the key with an
  X `grab_key` in another process (see `/tmp/fallback-test.sh` for the shape): that makes a
  conflict deterministic and proves the fallback walk without pressing anything. Real presses
  still need a human, or the XDG GlobalShortcuts portal (ADR 0007), which is testable over
  D-Bus.
- **A key held by another *X client* does not always block a Tauri shortcut registration.**
  Seeding `Ctrl+Alt+B` with a held grab did **not** fail registration in the app, while
  seeding `Ctrl+Alt+F9` with the same technique did. So an app-level key that appears free
  from `kglobalshortcutsrc` is not proof it is free; and conversely, a shortcut that registers
  is not proof the key reaches the app. Do not claim either without a real keypress.
- **Never read the game process's memory and never write to the game.** The only
  sanctioned live input is the SC2 Client API on `localhost:6119`. This keeps the app on
  the benign side of Blizzard's discretionary EULA clause.
- **Offline by default.** A feature that needs the network is opt-in, is fetched by an
  explicit user action, and caches its result into the local store. (See
  `docs/adr/0002`.)
- Do not use `as any`-equivalents to silence type errors, and do not add a dependency to
  work around a missing field — check the replay JSON first.

## Data boundaries and the overlay

- **The overlay's only inputs are the game clock and the matchup.** The user's rule is that the
  advisor must never react to the opponent or to the player — only to the predetermined times —
  because anything further treads into cheating. Reading the clock and the two races is the
  whole permitted surface: races are fixed before the first unit exists and never change, and the
  guide is a fixed list either way. So `#lib/components/BuildOrderGuide.svelte` may use
  `displayTime` and `matchupOf(players[0].race, players[1].race)`, and nothing else from the
  client API — no
  supply, no unit counts, no positions, and no reacting to anything the game does after it
  starts. Never add a suggestion, an alert, or an adaptation.
- **Replay analysis never touches the network; only `tools/refresh_build_snapshot.py` does.** The
  build library it writes is committed as `src/lib/builds.json` and inlined by a static import,
  so the app works offline. A scrape failure keeps the previous snapshot; writing an empty one
  would look current and be wrong.
- **A build's matchup is a `BuildMatchup` (`ZvX` is real); a game's is a `Matchup`.** They are
  deliberately different types, so widening the build type cannot quietly weaken the game type
  into allowing a wildcard a game can never produce.
- **`BuildStep.note` is load-bearing.** 19 steps in the bundled snapshot are named `Action` and
  carry their entire instruction in the note; without it they render as a bare meaningless
  `Action`.
- **Timeline panels share `cursorSecond`.** The feature list asks for the map to show where a
  battle happened when hovering a change in the graph, so the cursor is owned by the route page
  and passed down to every chart, never held per-chart.
- **Per-matchup presets resolve exact → `<own>vX` wildcard → first library build → sample.** See
  `resolvePresetBuild` in `src/lib/build-library.ts`. A preset naming a build a snapshot refresh
  removed falls through as if unset; erroring would leave the overlay blank at game start. The
  settings dropdown uses `exactMatchupBuilds`, not `buildsForMatchup`, so a wildcard the overlay
  already applies cannot be offered as a choice that changes nothing.

## Frontend

- Import from `#lib/...`, not `$lib/...`. SvelteKit 3 dropped the implicit `$lib` alias in
  favour of Node subpath imports, which must be declared in **both** `package.json`
  (`imports`) and `tsconfig.json` (`paths`) — declaring only one resolves at build time and
  fails typecheck, or the reverse.
- The build is a client-rendered SPA: `adapter-static` feedback plus `ssr = false` and
  `prerender = false` in the root `+layout.ts`. A route whose parameters only exist after
  indexing (such as `/analyze/[id]`) must **not** be prerendered — `adapter-static` treats
  a declared-but-unseen prerenderable route as a build error, so leaving the root rule
  alone is enough; do not add per-route opt-ins.
- The app is a single window whose *view* is swapped in `src/routes/+layout.svelte`: the
  build-order guide replaces the route children while the window is in overlay mode. A page
  must therefore never assume it is always mounted — anything that must survive the swap
  belongs above the layout, and the guide holds no state that the analysis pages need.
- When adding a Tauri plugin, add it in **three** places or it fails at runtime rather
  than build time: `src-tauri/Cargo.toml`, the `.plugin(...)` chain in
  `src-tauri/src/lib.rs`, and its permission in `src-tauri/capabilities/default.json`. A
  permission naming a plugin that is not a dependency is a hard compile error; a plugin
  added without its capability compiles and then fails on first call.
- Keep the capability scope minimal. The only network permission the app has is
  `http:default` limited to `http://localhost:6119/*` and `http://127.0.0.1:6119/*` — if
  a change needs a broader scope, it needs an ADR first.

## Updating this file

Whenever a code convention or hard-won platform constraint is decided or changed, record
it here in the same change. Append to the relevant section; add a new section rather than
burying a rule inside an unrelated one.

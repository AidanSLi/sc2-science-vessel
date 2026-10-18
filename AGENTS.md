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
  The working path is the XDG `GlobalShortcuts` portal (ADR 0007), and the two mechanisms are
  deliberately exclusive — registered both at once, one keypress toggles the window twice,
  because on Wayland the X11 grab still fires while our window has focus.

- **A portal binding with an empty trigger is a permanent failure, not a transient one.** When
  the compositor's approval dialog is dismissed, KDE registers the shortcut id with **no key**
  so it will never ask again — observed as `toggle-overlay=none,none,…` in
  `~/.config/kglobalshortcutsrc` and `unknown key ""` in the portal's own log. The app reported
  success while the key was dead for the life of the install. `bind_overlay_shortcut_via_portal`
  now checks `trigger_description()` for a non-empty value and returns `false` so the X11
  fallback runs. Do not remove that check, and do not assume re-running the app re-prompts — it
  does not; the user has to clear the entry or set a key in System Settings.

- **The portal resolves a host app's identity from its systemd scope plus a `.desktop` file.**
  `xdg-desktop-portal` refuses with `XDG_DESKTOP_PORTAL_ERROR_NOT_ALLOWED "An app id is
  required"` when it cannot determine one, and derives it from the unit name
  `app-<ApplicationID>-<random>.scope` (`xdp-app-info-host.c`). A binary run from a terminal
  inherits the terminal's id — ours showed up as `org.kde.konsole`, which is why the portal's
  binding landed under `[org.kde.konsole]` in `kglobalshortcutsrc`. A bundled `.deb`/`.rpm`
  installs the right `.desktop`; for development a
  `~/.local/share/applications/dev.aidansli.sc2-science-vessel.desktop` is enough.

- **The app is resident, so the frontend cannot be the thing that watches for a game.** A
  hidden WebKitGTK widget has its page timers suspended, so a `setInterval` poller in the UI
  goes silent exactly when the app is meant to be working. Game-start detection, the hint
  popup and the hotkey's mode rule therefore live in `src-tauri/src/game_watch.rs`, and the
  notification is sent and closed by hand because **Plasma ignores the timeout on a critical
  notification** (measured: still on screen at t+8s with `-t 4000`, and repeated sends stack).
  See ADR 0012 and ADR 0016.

- **An always-on-top window cannot cover an active fullscreen client.** KWin's
  `Window::belongsToLayer()` tests `isActiveFullScreen()` before `keepAbove()`, so a focused
  fullscreen window is on `ActiveLayer`, above `AboveLayer`. A window typed
  `_NET_WM_WINDOW_TYPE_CRITICAL_NOTIFICATION` is on `CriticalNotificationLayer`, which *is*
  above it — that is the whole reason the hint is a notification. Measured in pixels: with the
  probe active the overlay contributed 0; typed as a critical notification, a window was
  visible. See ADR 0011.

- **The frame offset is the reason the overlay drifts.** `get_geometry` on an X11 window is
  parent-relative, so an overlay positioned from it lands in the wrong place as soon as the
  game moves. `x11_overlay::find_game_window` uses `translate_coordinates` against the root for
  that reason. Game-window matching is by **title**, not `WM_CLASS`: the game is launched
  through Steam/Battle.net and Wine reports the launcher's class
  (`class=('steam_app_default','steam_app_default')`, title `'StarCraft II'`), so class
  matching either misses the game or matches half of Steam's helpers. Titles are ranked — an
  exact `StarCraft II`/`StarCraft 2` beats a prefix match like `StarCraft II — Dolphin`, which
  otherwise competes with the real game — and ties break on area.

## The process model: two binaries

- **The app is a watcher that owns a GUI child, not a resident web view.** Measured on the
  reference machine, idle and hidden: a resident Tauri app costs **259 MiB PSS** across three
  processes (`sc2-science-vessel` 123, `WebKitWebProcess` 101, `WebKitNetworkProcess` 34), where
  a dependency-free watcher costs **0.5 MiB**. That is ~500x, and no arrangement of a web view
  closes the gap. See ADR 0013.
- **The two roles are two executables, and that is deliberate.** `sc2sv-watch` (crate
  `src-tauri/watcher-bin/`, no dependencies) is the resident process; `sc2-science-vessel` is the
  window. They started as one binary chosen by `--gui`, which was wrong for a measurable reason:
  one ELF shares one set of shared libraries, so the watcher mapped WebKit and GTK it never
  called into — **23.6 MiB PSS instead of 0.5 MiB**. Do not merge them back to "simplify"; the
  split *is* the size fix. See ADR 0014.
- **The watcher's logic lives once, included by path.** `watcher-bin/src/main.rs` pulls
  `control.rs`, `game_process.rs`, `gui_control.rs`, `single_instance.rs` and `watcher.rs` out of
  `src-tauri/src/` with `#[path]`. A copy would drift, and the two roles would disagree about
  something invisible such as which process names count as the game. Both crates test the same
  files, which is why `cargo test` in `watcher-bin` runs 22 of the same tests.
- **`sc2sv-watch` is staged for the bundler by `tools/build_watcher.py`**, because Tauri can only
  bundle the binary named in `Cargo.toml` and `externalBin` is the one mechanism that installs a
  second executable beside it. The staged name must carry the host target triple or the bundle
  fails looking for a file that is not there. `--check` exists for CI to fail early.
- **The watcher finds the GUI by sibling lookup**, not by configuration: both are installed into
  the same directory. `SC2SV_GUI_EXECUTABLE` overrides that for the case where a packager
  separates them.
- **Quit from the tray also stops the watcher.** Otherwise the watcher sees its child exit, keeps
  running, and starts a fresh window the next time SC2 appears — the app would look as if it had
  ignored "Quit". `control::QUIT` / `control::WATCHER_CONTROL_NAME` are declared in
  `control.rs` rather than `watcher.rs` precisely because the GUI cannot include `watcher.rs`
  without dragging the whole supervisor in.
- **"Launch at login" writes the desktop entry itself, naming `sc2sv-watch`.** It was
  `tauri-plugin-autostart` until the split, and that plugin can only record
  `std::env::current_exe()` — which is necessarily the GUI, since the settings page asks from
  there. A login entry pointing at the GUI starts a window with no watcher behind it and no game
  detection at all. See `src-tauri/src/autostart.rs`; only Linux is implemented, and it returns
  an explicit error rather than silently doing nothing elsewhere.
- **The token on the path is the app, not the window.** `~/.local/bin/sc2-science-vessel` is a
  symlink to `sc2sv-watch`, and the `.desktop` entry's `Exec` names the watcher, because "run the
  app" means "be resident and show me a window".
- **The user's vocabulary is load-bearing: the *game* is the SC2 process, a *match* is one game
  inside it.** The watcher detects the game, by process name, and the GUI detects a match, from
  the client API's screen list. Do not conflate them: "is SC2 running" cannot be asked of the
  client API (it does not exist until SC2 starts), and "is a match being played" should not be
  guessed from process names when the client reports it exactly.
- **The watcher retires a GUI only if it started it.** `GuiOrigin::{Game,Player}` exists because
  the first version retired any GUI whenever SC2 was absent, so launching the app with no game
  open produced a window that vanished on the next poll. A player-started GUI survives the game
  exiting; a game-started one is killed with it, which is what returns the 259 MiB.
- **The GUI is spawned hidden when the game starts, and revealed by the match.** A game starting
  is not a match starting: the player is in the menus, and a window there would be the app
  interrupting them. `SC2SV_START_HIDDEN` carries that decision from the watcher to the child,
  because the watcher knows *why* the GUI is starting and the `start_hidden` setting does not.
  An absent variable means the setting applies (a shell launch, `tauri dev`).
- **Discovery and reveal are different from mode changes.** `reveal_window_mode_command` shows
  the window; `set_window_mode_command` does not. The match boundary uses the first; a manual
  mode change uses the second. Conflating them is the bug that pops the window back up over a
  player who hid it mid-match.
- **`tauri build --debug --no-bundle` embeds the frontend; `cargo build` does not.** A
  cargo-built binary points its web view at `localhost:1420`, so with no Vite running the window
  shows `Could not connect to localhost: Connection refused` and — this is the expensive part —
  the frontend never runs at all, so `/ui` is never polled, the mode never changes, and a
  lifecycle test silently passes while testing nothing. `tools/install_dev.sh` builds through
  `pnpm tauri build`, which is the only command that produces a runnable artefact. Check with
  `strings <binary> | grep -c localhost:1420` — 0 when embedded.
- **The watcher's control channel and the GUI's are separate ports** (`watcher`, `gui`), each
  published under `$XDG_RUNTIME_DIR`, so a second launch reaches the right process and neither
  can read the other's file.
- **Single-instance is a lock, not a pid file.** `std::fs::File::try_lock` (stable since Rust
  1.89) is released by the kernel on any exit including SIGKILL, so there is no stale state to
  detect and "is it running" is one atomic operation.
- **Never run `tools/install_dev.sh` with `sudo`.** It installs into `~/.local` for the current
  user, so as root it writes to `/root/.local`, where no login session looks. Modern rustup also
  puts the toolchain in the *user's* `~/.cargo/bin`, which root's `PATH` does not have, so
  `sudo tools/install_dev.sh` dies at the first `rustc` with `rustc: command not found` — a
  message that names neither sudo nor the toolchain. The script now refuses when `EUID` is 0.
  Anything that needs to work from a GUI-launched or non-interactive shell (no profile sourced)
  must resolve tool locations itself rather than trusting `PATH`: `install_dev.sh` prepends
  `~/.cargo/bin` when `rustc` is absent and `~/.local/share/pnpm/bin` when `pnpm` is. Verify that
  path with `env -i HOME=$HOME PATH=/usr/bin:/bin bash tools/install_dev.sh --no-build`, which is
  what a desktop launcher actually looks like.

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
- **A transparent window paints nothing from `html`/`body`; only real elements reach the
  screen.** `tauri.conf.json` declares the window `"transparent": true` so the overlay can sit
  over the game, and under a transparent webview WebKitGTK does not paint a `background-color`
  set on the document roots — a page that is text on transparent roots paints *nothing* (0 of
  14,362 expected text pixels). Each mode therefore gets a full-viewport block element, both
  `position: fixed; inset: 0` because a flow-layout `height: 100%` div also measured 98.4%
  alpha-0: `.app-surface` for analysis (`src/routes/+layout.svelte`) and `.guide-panel` for
  overlay. Verify with `XGetImage` on the window's own buffer, **not** a screenshot — the page
  reports correct layout while the pixels are absent, and a screenshot cropped to the window's
  rectangle happily captures whatever fullscreen window is stacked on top of it. See ADR 0017.
- **The guide's step list must have a styled scrollbar.** The GTK-themed default scrollbar is
  the only part of the page WebKit hands to its own composited layer under the CPU path
  (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, ADR 0005), and everything outside that layer is dropped:
  the overlay filled 4.2% of its window (a 21x457 strip at the right edge) with every step row
  invisible. `.guide-steps::-webkit-scrollbar` in `src/app.css` restores it to 100%; any custom
  scrollbar style works, so keep the bar narrow rather than hidden, and never reach for
  `overflow: hidden` — it paints but truncates the build order, which is the whole overlay.
- **`--guide-veil` needs a document-level default.** It is set inline from the component's
  `panelOpacity`, but `html.is-transparent-window { --guide-veil: 0% }` must exist too: without
  a value the `color-mix()` declarations are invalid at computed-value time and are dropped,
  which is how the guide lost its header/footer rules and row surfaces.
- **Mode and visibility are independent, and conflating them is a bug.** `apply_window_mode`
  takes a `Visibility` (`Shown` for the hotkey, `AsIs` for an automatic change). Without this,
  hiding the app mid-game and then finishing the game pops the window back up uninvited over a
  player who asked for it to be gone. Re-centring and `set_focus` are likewise confined to
  `Visibility::Shown`: an automatic scan of client state must never move the window the player
  placed or pull focus out of a running game. The rule is `should_reveal()`, with tests.
- **The hotkey chooses the mode it reveals, and it chooses it in Rust.** The user's rule: the
  hotkey brings up the **analysis window while SC2 is running but no match is on**, and the
  guide only during a match. A hidden WebKitGTK webview has its page timers suspended, so the
  frontend's poll cannot answer at the moment the key is pressed — the stored mode is frozen at
  whatever the last match left behind, which presents as a click-through guide over the menus.
  The rule is `game_watch::hotkey_mode(clock, screens)`: `/game` unreadable or `isReplay` →
  analysis, `/ui` unreadable → **no change** (`None`), empty `activeScreens` → overlay, any
  non-empty list → analysis. It names no individual screen, deliberately: every non-empty list
  means "not playing", so naming names would only duplicate `game-mode.ts`'s vocabulary. See
  `docs/adr/0016-the-hotkey-picks-its-mode.md`. `show_window` keeps the older meaning ("show in
  the stored mode") for the watcher's *click-the-icon-again* request; only the hotkey re-decides.
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
  `/game` says what the game is *doing*. **The transition was later caught on this machine**,
  in `/tmp/ui-transitions.log` (logged by `/tmp/uisample.sh`, which appends only when the screen
  string changes): `ScreenLoading/ScreenLoading` at 22:47:41, then at 22:47:44
  `["ScreenBackgroundSC2…","ScreenNavigationSC2…","ScreenForegroundSC2…","ScreenScore/ScreenScore","ScreenCustom/ScreenCustom"]`,
  then `Error 404` three seconds later as the client exited. So `ScreenScore` is confirmed
  against this machine's own build, not only inferred from another project's traces.
- **A game that ends takes the client API down with it.** After the match the endpoint is
  reachable for only a few seconds and then answers `Error 404: Not Found` (captured at
  22:47:49, five seconds after the score screen appeared). The mode switch therefore has to
  happen on the `ScreenScore` reading and cannot rely on the score screen staying readable — by
  the time `/ui` answers 404, `live.status` is `unavailable`, `decideMode` returns `null`, and
  the window keeps whatever mode it had. That is the correct behaviour (nothing should flip on a
  failed request), but it means the transition must be caught in the seconds while it lasts.
- **`screens: null` (could not read `/ui`) is NOT the same as `screens: []` (in a game).** In
  `src/lib/client-api.ts` `fetchActiveScreens` returns `null` on failure; `decideMode` returns
  `null` for it and the view is left alone. Collapsing the two would put a build-order guide
  over the player's menus on a transient HTTP failure.
- **`isActiveFullScreen()` is focus-conditional, so a fullscreen test that does not activate the
  fullscreen client measures the wrong thing.** KWin promotes a client to `ActiveLayer` only while
  it is fullscreen **and** active. Starting the app after a stand-in game leaves the app holding
  focus, the "fullscreen" client drops back to `NormalLayer`, ordinary keep-above wins, and the
  overlay measures as visible **with no rule installed** — a false positive that reads as proof of
  fullscreen support. Activate the fullscreen client *after* the app is up (`qdbus6 org.kde.KWin
  /WindowsRunner org.kde.krunner1.Run <id> ""`), then screenshot and count. Measured on a
  3840x2160 screen with an active fullscreen client: **0 non-backdrop pixels without the KWin
  rule, 114000 with it**. Settle stacking claims with pixels (`/tmp/fs/countpix.py`,
  `/tmp/fs/activetest.sh`), never with a layer number alone — a layer number is a compositor
  report, not evidence that an image reached the screen.
- **The automatic mode switch is verified against a scripted client API, not only in unit tests.**
  `/tmp/fs/fakeapi.py` (a throwaway, not in the repo) serves `/ui` and `/game` on port 6119 in a
  scripted sequence — in-game `[]`, then `ScreenScore`, then a second game and score — and the
  window geometry read back from KWin once a second shows the switch happening on its own
  (`304x240` overlay ↔ `1024x684` analysis). Worth repeating after any change to `decideMode`,
  the poll loop, or `apply_window_mode`, because the unit tests cover the decision and this covers
  the wiring. **Bind the fake API and wait until it answers before starting the app**, or the
  first poll races it and the app can latch the wrong mode.
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
- **Never call `set_resizable(false)` on the overlay: GTK reads it as "size me to my
  content".** A non-resizable GTK window takes its size from its child's natural size, which
  beats both an explicit `set_size` and pinned min/max geometry hints. Measured on cold starts
  into overlay mode: with the flag set the window settled at **1280x2108** (the WebView's
  natural size, KWin reporting 1024x1686), and without it the intended **380x300 / 304x240**
  every time. The overlay is click-through anyway, so its edges cannot be dragged and the flag
  buys nothing. Beware the trap that hid this: the app's own `inner_size()` read `1280x820`
  right after `set_size`, and the wrong size appeared a few milliseconds *later* as GTK
  re-applied the content size — so a read immediately after the call looked like success.
- **A geometry request against an UNREALISED window is a race, not a guarantee.** GTK needs a
  realised `GdkWindow` before `set_size`/`set_position`/click-through mean anything, and a
  request issued before the window is first mapped is queued against a window that does not
  exist yet, so it is silently lost. This was measured directly on start-up into overlay mode:
  `inner_size()` read `1280x820` immediately after `set_size(380x300)` *and* after `show()`, and
  only a `set_size` repeated **after** the show took effect (`380x300`). The observable symptom
  was intermittent — 6 consecutive cold starts all came up at the analysis size, while a
  hand-built test had shown the right size only because its window had already been shown once.
  **Every** geometry request in `apply_window_mode` therefore comes after `show()`, in both
  branches; this is the same underlying issue as the click-through/always-on-top ordering
  (ADR 0007), and the rule is always "map first, then request". Confirm a sizing fix over
  several cold starts — never from one run and never from the call returning `Ok`.
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
- **XTEST *does* reach XGrabKey grabs through Xwayland, and this was wrongly believed
  otherwise for a long time.** The deciding test is a process that grabs the key on the root
  window, verifies a *second* grab is refused with `BadAccess` (proving the grab is really in
  place), then injects the same combination with `Xlib.ext.xtest.fake_input`. The grab fires
  (an event with `type=2` KeyPress arrives). The earlier "XTEST cannot reach grabs" conclusion
  came from a `BadAccess` on the test's own grab — which means something *else* already held
  the key, i.e. the opposite of free.
- **A "the hotkey does nothing" result is almost always the wrong key, not a broken hotkey.**
  `register_overlay_shortcut` walks a fallback list when the stored key is taken and *persists*
  the substitution, so the app can be listening on a key the settings file no longer names.
  Read `~/.config/dev.aidansli.sc2-science-vessel/settings.json` **and** the log line
  `registered the global shortcut <key>` before pressing anything. Two long detours were
  caused by pressing `Ctrl+Alt+U` while the app had bound `Ctrl+Alt+O`.
- **Proving a grab exists**: a second `grab_key` on the same key/modifiers is refused with
  `BadAccess` while a holder is running (self-test: run a holder, expect `TAKEN`). A `FREE`
  reading is only meaningful with that self-test in place.
- **An override-redirect window cannot be hidden while it is being re-flipped.** The
  unmanaged flip is a GTK `hide()` + `show()` pair, and it preserves "was visible" so that
  stacking changes never alter visibility. The hotkey path therefore hides first and passes
  `restore_visibility = false` to the release; without that the flip's own `show()` wins the
  race and the overlay comes straight back (measured: three presses all reporting
  `is_visible=true`).
- **Never drive GTK from a background thread.** `gtk::main_iteration_do` and friends panic with
  `GTK may only be used from the main thread.` A "synchronously hide the window" helper built
  on them is not viable from the global-shortcut thread.
- **The stacking watcher and the hotkey must share one lock** (`OVERLAY_STACK_LOCK`). The
  watcher pokes the window every 400 ms; a hide that lands between the flip's hide and show is
  undone by it.
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

## Packaging and platform support

- `docs/packaging.md` is the record; read it before touching the bundler or the CI workflow.
- Tauri's Linux `--bundles` accepts exactly `deb`, `rpm`, `appimage` — there is **no pacman
  target**, so Arch is served by the AppImage (or a hand-written AUR `PKGBUILD`). Never plan on a
  separate "Arch build".
- The Python sidecar must be frozen **before** the Tauri build (`tools/build_sidecar.py`), because
  `externalBin` is resolved at bundle time. Three things fail at runtime rather than build time if
  done wrong: the filename must carry the host target triple (ask `rustc -vV`, do not guess),
  `--collect-data sc2reader` is mandatory (three JSON data files), and the PyInstaller entry point
  must be a module shim rather than `sc2sv/cli.py` (relative imports).
- A missing sidecar makes the build fail loudly (`resource path ... doesn't exist`); it does not
  produce a bundle without it. Keep it that way — do not add a fallback that silently skips it.
- Keep `src-tauri/tauri.conf.json`'s `targets` free of `rpm`: `rpmbuild` is not installed on the
  reference machine, so a plain local `pnpm tauri build` would fail for a reason that has nothing
  to do with the change being made.
- Platform-specific window code lives behind `src-tauri/src/platform/`. `lib.rs` must never name a
  platform type: adding a platform means adding a `platform` implementation, not another `#[cfg]`
  in the watcher. `AnchorOutcome::Unsupported` and `::NoGame` are deliberately different — the
  first stops the watcher, the second keeps it alive.
- `CLAIM_ALWAYS_ON_TOP` is `false` on Windows. A topmost window is above every non-topmost window,
  which is *stronger* than the requirement ("above the game, able to go behind a window the player
  raises over it"); restacking expresses the requirement exactly.
- On Windows `SetWindowPos`'s `hWndInsertAfter` names the window that *precedes* the positioned
  window in Z order, so passing the game's handle puts the overlay **below** the game. Pass
  `GetWindow(game, GW_HWNDPREV)` (the window currently above the game), falling back to
  `HWND_TOP`. Always include `SWP_NOACTIVATE` or the restack steals focus from the game.
- Windows code is written from the documented API but has never been run on Windows: there is no
  Windows machine here and no Windows target links on Linux. Do not describe it as verified, and
  treat the first real Windows run as the verification step.
- **`notify-rust`'s `NotificationHandle` methods differ per backend, so any use of one must be
  `cfg`-gated.** Only the xdg backend implements `NotificationHandle::close`
  (`notify-rust-4.18.2/src/xdg/mod.rs:214`); the Windows handle has `wait_for_action`,
  `wait_for_response`, `on_close` and the macOS handle has none either. Calling `close()`
  unconditionally compiles on Linux and fails on Windows with
  `error[E0599]: no method named `close` found for struct `NotificationHandle``. The gate in use is
  `#[cfg(all(unix, not(target_os = "macos")))]`.
- **`tauri-build` needs `icons/icon.ico` to build for Windows at all** — it is embedded in the
  executable's resource section. A repo with only PNG icons builds fine on Linux and fails on
  Windows with `` `.../icons/icon.ico` not found; required for generating a Windows Resource
  file``. `pnpm tauri icon src-tauri/icons/icon.png` regenerates the whole platform set; keep
  `icon.ico` and `icon.icns` listed in `tauri.conf.json`'s `bundle.icon`.
- **Rehearse a Windows-only compile error without a Windows toolchain**: a tiny scratch crate
  containing just the offending module plus its dependency can be `cargo check --target
  x86_64-pc-windows-msvc`'d on Linux. `notify-rust` and the `windows` crate need no MSVC linker
  to *type-check*; only `ring` does, and it is the one thing that stops the full app from being
  cross-checked. This caught nothing here that CI had not, but it turns a 20-minute CI round trip
  into a 5-second local one.
- The app is called **SC2 Science Vessel** (`productName`, the window title, and
  `notify::POPUP_TITLE`), while the binary, the `.deb` package name, and the D-Bus/desktop
  identity stay `sc2-science-vessel`. Both spellings are load-bearing: KWin matches `WM_CLASS`
  case-sensitively for the overlay rule, so `startupWMClass`/the rule class must not be renamed to
  the display name. Consequence of the space in `productName`: the deb and its
  `usr/share/applications/SC2 Science Vessel.desktop` both carry spaces in the filename (no other
  `.desktop` on the reference machine does). `desktop-file-validate` passes, and `Exec=` is still
  the binary name, so this is legal — but do not 'fix' it by renaming the binary, because that is
  what `Exec`, `externalBin` resolution, and the X11/Win32 window matching all depend on.
- Package names: `Package: sc2-science-vessel` with `Depends: libayatana-appindicator3-1,
  libwebkit2gtk-4.1-0, libgtk-3-0` — Debian's names, so a `.deb` will not install on Arch even
  though the binaries run. Arch gets the AppImage.

- **A window that accepts focus demotes the fullscreen game it covers.** KWin moves an active
  fullscreen client from `ActiveLayer` (5) to `NormalLayer` (2) as soon as any other window becomes
  active. An overlay that takes focus therefore makes the app's own "is the game active" check
  read `false`, which releases the unmanaged state, which drops the overlay behind the game, which
  makes the game active again — a visible oscillation (~25 polls one way, ~14 the other). Fix:
  `platform::refuse_focus()` (`set_accept_focus(false)` + `set_focus_on_map(false)`) must run
  **before** `show()`, because those flags only affect the focus decision at map time. Analysis
  mode calls the inverse; a stale refusal would leave it unable to raise or accept a keystroke.
- **An override-redirect window is the only unprivileged way above an active fullscreen window**,
  and it is per-window rather than per-user. KWin's `Window::belongsToLayer()` returns
  `OverlayLayer` for `isUnmanaged()` *before* consulting `isActiveFullScreen()`, whereas
  `keepAbove` can only reach `AboveLayer`, which is below `ActiveLayer`. Measured over a
  fullscreen client: managed keep-above **0 %** visible, KWin rule `layer=overlay` 13 %,
  override-redirect **8.44 %** (exactly its own area). Do not replace this with an always-on-top
  claim, and do not make the window unmanaged while the game is merely *fullscreen* — only while
  it is the *active* window, or the overlay will cover whatever the player deliberately raised.
- **`override_redirect` cannot be changed on a live X window.** Flipping a mapped window is
  ignored (the window manager decided at first sight), and flipping while unmapped in raw X leaves
  the window mapped-but-unpainted. Only GDK's own `hide()` → `set_override_redirect()` → `show()`
  works, because it tears the X resource down and recreates it. Preserve visibility across the
  flip: the flip is about *stacking*, and must never change whether the player can see the window.
- **Compare the game window by title, not by X id.** Wine gives the game several viewable
  fullscreen windows with the same title (five on the reference machine), and the one the
  compositor reports as active is frequently not the one the anchor finder picked. Comparing ids
  reported "the game is not active" while the player was demonstrably playing.
- **The overlay is above an active fullscreen game with no user configuration on any EWMH
  desktop**, X11 included, because override-redirect is a core X11 concept. The optional KWin rule
  in `tools/overlay_fullscreen_rule.sh` remains as an escape hatch, not the mechanism.
- **`always_on_top` is owned by exactly one place: the stacking watcher.** It is a compositor
  *layer*, so `AboveLayer` beats any ordinary window regardless of the X stacking order, and the
  watcher tracks its value to avoid re-sending it every tick. Any other code setting the flag
  desynchronises that belief, and the flag then never gets cleared — measured: the overlay sat at
  `layer=3 keepAbove=true` above a window the player had raised over the game (`layer=2`), which is
  the exact opposite of the requirement. `claim_unmanaged` deliberately does not set it: an
  override-redirect window needs no claim at all.
- **Releasing the override-redirect state re-shows the window, which raises it to the top of the
  stack.** The caller must re-anchor afterwards or the overlay lands *above* whatever the player
  just raised. Verified in the raw X stacking order: before the fix `game → RAISED → overlay`,
  after `game → overlay → RAISED`.
- Screenshot a fullscreen overlay check with the game **actually running**: the fake fullscreen
  client cannot exercise the `game_active` path because it takes X input focus directly, which
  never updates `_NET_ACTIVE_WINDOW`, so the app reads `game_active=false` against it forever.
  `/tmp/fs/realapi.sh` prints `_NET_ACTIVE_WINDOW`, input focus, and every `StarCraft`/`SC2`
  window with markers, which is what makes that claim checkable.

## Updating this file

Whenever a code convention or hard-won platform constraint is decided or changed, record
it here in the same change. Append to the relevant section; add a new section rather than
burying a rule inside an unrelated one.

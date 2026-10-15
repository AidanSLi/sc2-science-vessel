# sc2-science-vessel — conventions

## Language and toolchain

- **Python 3.11+** for the analysis sidecar. It depends on `sc2reader` (1.9.0 tested) and
  `spawningtool` (3.0.0 tested); both are actively maintained and `spawningtool` is MIT.
- **TypeScript + SvelteKit + Tailwind** for the UI.
- **Tauri 2** for the shell. Do **not** design anything that requires Tauri 3 / GTK4:
  the GTK4 migration (wry#1474) has been stalled since 2025-01-30 with no landing date.
- Package manager: **`uv`** for Python (`uv venv`, `uv pip install`); **`npm`** for JS.
  `pnpm` is not installed on the reference machine.

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

## Hard rules learned the hard way

- **The replay clock is in game seconds.** `frames / 16` = in-game seconds; real seconds
  at `Faster` (the ladder default) = in-game seconds × 1.4 (measured across 722 replays).
  Never mix the two. Anything time-related that ignores this produces timings that are
  40% wrong.
- **`PlayerStatsEvent` is sampled every 10 game seconds.** Never claim sub-10-second
  precision for a stat derived from it; use per-unit tracker events when you need more.
- **`UnitDiedEvent.unit_type_name` is `None`.** Join deaths to births on `unit_id` to
  recover the unit type. Do not trust the field.
- **Filter cosmetic noise from `UpgradeCompleteEvent`** (`Spray*`, `Reward*`, `Dance*`),
  and neutral/map spawns from `UnitBornEvent` (`MineralField*`, `VespeneGeyser`,
  `InvisibleTargetDummy`, `XelNagaTower`). Both pollute counts badly.
- **Never read the game process's memory and never write to the game.** The only
  sanctioned live input is the SC2 Client API on `localhost:6119`. This keeps the app on
  the benign side of Blizzard's discretionary EULA clause.
- **Offline by default.** A feature that needs the network is opt-in, is fetched by an
  explicit user action, and caches its result into the local store. (See
  `docs/adr/0002`.)
- Do not use `as any`-equivalents to silence type errors, and do not add a dependency to
  work around a missing field — check the replay JSON first.

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
- Ask Tauri for windows through the JS API where possible (`getByLabel` then a builder
  fallback). A second window created in Rust *and* JS duplicates the route and the flags.
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

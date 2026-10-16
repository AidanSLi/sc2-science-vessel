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

Skeleton. The assessment, ADRs, and conventions are written, and the app runs end to end:
SvelteKit 3 + Tailwind 4 in a Tauri 2 shell, with the always-on-top click-through overlay
window and its global-shortcut toggle. Nothing reads a replay yet — the next milestone is
the Python sidecar (Phase 1).

The SC2 Client API clock on `localhost:6119`, which the overlay's timer depends on, is
still **UNVERIFIED against a running game**; only one replay exists on the reference
machine and StarCraft II is not installed there.

## Running it

Prerequisites: Node 22+, pnpm 12+, Rust 1.77+ with the Tauri 2 system dependencies (on Arch:
`webkit2gtk-4.1`, `gtk3`), and — for Python work — `uv`.

```sh
pnpm install
pnpm check         # svelte-check, zero warnings is the bar
pnpm build         # static SPA into build/
pnpm tauri dev     # the desktop app
```

The global shortcut is X11-only (Tauri issue 3578); on Wayland open the overlay from the
Builds page instead. This does not change the overlay window's behaviour.

### Linux desktop notes

The WebKitGTK DMABUF renderer fails on some Mesa/driver combinations, two ways, neither with a
useful error. Under Wayland the process dies outright:

```
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
```

Under X11 the window opens but nothing is presented, while stderr repeats
`Failed to create GBM buffer of size 1280x820: Invalid argument`.

**Nothing to do** — the app disables the DMABUF path itself at start-up and logs one line
saying so. `WEBKIT_DISABLE_DMABUF_RENDERER=0` restores it deliberately if your drivers
handle it. See `docs/adr/0005-disable-webkitgtk-dmabuf-renderer.md`.

### The overlay over a fullscreen game (KDE Plasma Wayland)

The overlay sits above the game and follows it while the game is a normal window. It cannot do
so while the game is **fullscreen**: KWin puts a focused fullscreen window on its `ActiveLayer`,
which is above every layer an always-on-top window can reach, and stacking order sorts purely by
layer index. While that is the case the app drops its always-on-top claim rather than pretend,
so the overlay is behind the game but can be raised and read.

If you want it genuinely over a fullscreen game, install the optional KWin rule:

```bash
./tools/overlay_fullscreen_rule.sh install   # then restart the app
./tools/overlay_fullscreen_rule.sh remove    # to undo
```

This lifts the window into KWin's `OverlayLayer`, which does cover fullscreen windows — but also
every other application's, so it is a desktop-wide change and is never applied automatically.
The alternative is to run SC2 in **borderless/windowed fullscreen**, where everything works with
no configuration. See `docs/adr/0011-yield-the-overlay-to-a-fullscreen-client.md`.

### If the window looks blank

The main window is created hidden and is shown by the first page that paints, so it should
never appear empty. If it does:

- **A first run on a cold cache is slower**, because Vite logs `Forced re-optimization of
dependencies` and takes several seconds to serve anything. The window simply does not appear
until it is ready, rather than appearing blank.
- **If no window appears at all**, the frontend never signalled readiness and the 10-second
watchdog has not fired yet. Give it a moment, then check the dev server's output for
`200 GET /`.
- **A leftover `vite` holding port 1420** makes the run fail at start-up
  (`Error: Port 1420 is already in use`). Check with `ss -ltnp | grep :1420`.

## Layout

```
docs/assessment.md   feasibility study — read this first
docs/adr/            architecture decision records
AGENTS.md            conventions for agents working in this repo
src/                 SvelteKit frontend (routes/(app) is the main window, routes/overlay the overlay)
src-tauri/           Rust shell: windows, the global shortcut, sidecar lifecycle
```

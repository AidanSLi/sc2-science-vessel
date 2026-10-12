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

Prerequisites: Node 22+, Rust 1.77+ with the Tauri 2 system dependencies (on Arch:
`webkit2gtk-4.1`, `gtk3`), and — for Python work — `uv`.

```sh
npm install
npm run check      # svelte-check, zero warnings is the bar
npm run build      # static SPA into build/
npm run tauri dev  # the desktop app
```

### Linux desktop notes

The WebKitGTK DMABUF renderer is broken against some Mesa/driver combinations. Both
failure modes were seen on the reference machine (a KDE 6.7 Wayland session, Mesa 25.x):
a hard `Error 71 (Protocol error) dispatching to Wayland display`, and a window that opens
with a blank webview and `Failed to create GBM buffer of size 1280x820: Invalid argument`
repeated on stderr. Either way the app looks dead and nothing is logged that points at the
cause:

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 npm run tauri dev
```

This does not affect the overlay window's behaviour. The global shortcut is X11-only
(Tauri issue 3578); on Wayland the overlay is opened from the Builds page.

## Layout

```
docs/assessment.md   feasibility study — read this first
docs/adr/            architecture decision records
AGENTS.md            conventions for agents working in this repo
src/                 SvelteKit frontend (routes/(app) is the main window, routes/overlay the overlay)
src-tauri/           Rust shell: windows, the global shortcut, sidecar lifecycle
```

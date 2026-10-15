# 5. Disable WebKitGTK's DMABUF renderer by default on Linux

**Status:** accepted
**Date:** 2026-10-14

## Context

WebKitGTK 2.54.1's DMABUF renderer fails on the reference machine's driver combination: an
NVIDIA RTX 4070 (driver 615.78.08) alongside an AMD Raphael iGPU, on KDE Plasma 6.7.5 /
kwin 6.7.5 Wayland. DMABUF is a zero-copy texture path between the web process and the
compositor.

**Verified:** on Wayland, with no environment variable set, the app dies during start-up
with

```
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
```

With `WEBKIT_DISABLE_DMABUF_RENDERER=1`, it starts and renders normally. This was measured
by running the same built binary four ways (Wayland, Wayland + the flag, X11, X11 + the
flag); only the two no-flag... the flag-less Wayland case dies, and all three others stay
alive and render.

**Not verified:** that the DMABUF renderer explains a *persistent blank window*. On X11 the
binary logs `Failed to create GBM buffer of size 1280x820: Invalid argument` on every frame
while still rendering correctly, so those messages are not sufficient to diagnose a blank
window. A separate report of a blank window under Wayland **with the flag already set** could
not be reproduced at all (see below), so the cause of that report is still open — see
"Still open" below rather than assuming this ADR covers it.

## Decision

Set `WEBKIT_DISABLE_DMABUF_RENDERER=1` from inside the process, in `main`, before Tauri
initialises WebKit, when the user has not already set it.

It is done in Rust rather than in `src-tauri/.cargo/config.toml` because a cargo `[env]`
block reaches `cargo run` and `tauri dev` but **not** a bundled deb/rpm/AppImage or a
`.desktop` launch, which would leave every installed user crashing on start-up. A single
source of truth also keeps the start-up log line honest: with the variable injected by
cargo, `main` would see it already set and stay quiet, so it would be unclear which
mechanism was responsible.

An explicit `WEBKIT_DISABLE_DMABUF_RENDERER=0` is respected, and the app prints one line at
start-up when it applies the workaround. A rendering path change that happens silently is
the kind of thing that becomes an unfixable mystery later.

## Consequences

- `pnpm tauri dev` and the packaged app both work with no environment variables, and the
  Wayland crash is gone.
- Slightly lower compositing performance. Irrelevant here: the main window repaints on user
  interaction, and the overlay repaints about once a second.
- **This is a workaround for an upstream bug.** When webkit2gtk's DMABUF path works on this
  hardware, delete the block in `src-tauri/src/main.rs` and this ADR.
- It only affects binaries built from this repo. Other WebKitGTK apps on this machine need
  their own treatment.

## Still open: a blank window that this does not explain

A report of a **blank window while the flag was already set** remains unreproduced. Four
attempts all rendered correctly, including a cold run with the Vite dep cache, the
`.svelte-kit` cache and the WebView cache all deleted. The most likely explanation is the
start-up transient rather than a defect: Tauri shows the window before the frontend has
loaded, and on a first run Vite logs `Forced re-optimization of dependencies` and takes
several seconds to serve anything, so a newly cloned project shows a blank window for longer
than a warm one.

If it recurs, distinguish the two cases before changing anything:

- **Transient:** the window fills in on its own within ~15 seconds. Nothing to fix.
- **Stuck:** it is still blank a minute later. Then check, in this order:
  1. Does the dev server show `200 GET /` in its output? If not, the webview never asked
     for the page and the problem is in the Rust shell or the dev URL.
  2. Does `curl -s http://localhost:1420 | head -c 200` return HTML? If not, Vite is not
     serving and the window has nothing to load.
  3. Is port 1420 already held by a leftover `vite` from an earlier run? A stale server
     makes a fresh `tauri dev` fail at start-up with
     `Error: Port 1420 is already in use`, and a stale *window* from an earlier run looks
     exactly like a fresh blank one.

That third point cost real time during this investigation and is worth stating plainly:
three separate readings were wrong because an app instance from a previous run was still on
screen. Cleanup must kill by process name **and** by listening port:

```sh
pkill -9 -f "target/debug/sc2-science-vessel"
ss -ltnp | grep :1420    # whatever holds this is Vite; kill it
```

`pkill -x sc2-science-vessel` silently does nothing — Linux truncates `comm` to 15
characters, so the name is `sc2-science-ves`. And a `pkill -f` pattern that also matches the
invoking shell's own command line kills the harness; it did so four times here.

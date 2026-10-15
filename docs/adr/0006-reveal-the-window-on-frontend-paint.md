# 6. The main window is shown by the frontend, with a watchdog behind it

**Status:** accepted
**Date:** 2026-10-14

## Context

A blank window was reported. It could not be reproduced: over a dozen runs — dev and
packaged, Wayland and X11, warm and cold caches, and through an interactive zsh — the app
always rendered. The report was nonetheless actionable, because it pointed at something
genuinely wrong with the start-up sequence rather than at a specific defect.

`tauri.conf.json` declared the main window as visible. Tauri therefore showed it as soon as
it was created, which happens **before** the frontend has loaded: the shell is up, the window
is on screen, and the webview has not yet fetched, parsed or executed anything. On a warm run
that gap is imperceptible. On a cold one it is not — a first run logs
`Forced re-optimization of dependencies` and Vite takes several seconds to serve anything, so
the user watches an empty window for that whole time. On a slower machine, or with a large
dependency graph, that is long enough to look like a broken app rather than a slow one.

That is worth fixing on its own terms: a window that is shown before it has content is a
window that will eventually be seen empty, and the correct number of visibly blank windows is
zero.

## Decision

Declare the main window `visible: false`, and show it from the point where there is something
to see:

1. **The frontend asks for it.** The root layout calls a `frontend_ready` command from
   `onMount`, which fires after the initial render. It lives in the layout rather than in a
   page so no route can forget it.
2. **A watchdog shows it anyway.** `setup` spawns a thread that sleeps ten seconds and then
   shows the window if it is still hidden.

Both paths were verified rather than assumed. With the frontend healthy the window appears
after ~8s; with the frontend killed at 4s so it can never paint or signal, the watchdog shows
the window at ~12s. The failure mode is therefore "the window is late", never "the window
never appears" — which matters, because a hidden window is more dangerous than a blank one:
a blank window is visible and diagnosable, while a hidden one makes the app look like it
failed to start.

## Consequences

- No flash of empty window, on any machine, including cold caches.
- Ten seconds is long enough not to fire during a normal cold start and short enough not to
  look hung. It is a ceiling on how long a *totally* broken frontend can look like an
  unstarted app.
- The reveal signal must go through Tauri's IPC (`invoke`). A hand-rolled
  `window.__TAURI__` check would be wrong: that global is only injected when
  `app.withGlobalTauri` is enabled, which this app does not enable. `isTauri()` in
  `src/lib/client-api.ts` tests for `__TAURI_INTERNALS__` instead, which is always present,
  and is what guards the dynamic import so the same code runs in a browser tab.
- If the watchdog ever needs to move, it must not be reordered to run *after* awaiting the
  frontend signal. Its entire purpose is to be the path that does not depend on the frontend.

## Not addressed

This does not explain the originally reported blank window, which was reported *with*
`WEBKIT_DISABLE_DMABUF_RENDERER=1` already set and could not be reproduced in any
configuration. It removes the most likely cause — a window presented before it had content —
and guarantees the app can never present a window that has never painted anything. If it
recurs, `docs/adr/0005-disable-webkitgtk-dmabuf-renderer.md` has the ordered checks for
distinguishing a stuck webview from a start-up transient.

# ADR 0019 — A transparent web view drops its buffer on incremental repaints

- Status: accepted
- Date: 2026-10-19
- Deciders: flashandfoul
- Supersedes: nothing; completes ADR 0017 (how the modes are painted) and ADR 0018 (what makes
  each mode's web view transparent)

## Context

Three faults were fixed and verified, and then a live match produced two more reports:

1. The overlay "appears at the centre of the screen", and pressing the hotkey "briefly moves it to
   the correct position before it snaps back to the centre".
2. Inside a match only the status line is visible — "the rest of the text, including the actual
   build steps, is completely invisible" — while outside a match the same build was confirmed
   correct.

Both reproduced. The window sat at `(1730,754)` (screen centre rather than its anchor) and its own
ARGB buffer held **23,640 painted pixels of 228,000 (3.7 %)**, then **8,360 (3.7 %)** on a re-test,
in a single full-width 22 px band at `y=73..94`.

That band is the decisive clue: it is exactly the status line, and the status line is the only
part of the guide that *changes* — it carries the game clock, which ticks once a second. A bounding
box alone cannot distinguish "the panel is short" from "everything except one band is missing",
which is why `/tmp/rowprofile.py` (per-row painted counts) settled it.

## What was ruled out

Each of these was measured and is **not** the cause:

- **The override-redirect flip.** `set_unmanaged` does `hide()` → `set_override_redirect()` →
  `show()`, so the whole window and its `GdkWindow` are torn down and recreated. A fresh managed
  run reproduced the 3.7 % band without any flip, so the flip is not required for the fault.
- **The window being moved/centred.** Repositioning is a separate, real bug (see below); it does
  not affect painting.
- **`set_background_color` in the overlay branch.** Running with `SC2SV_OVERLAY_OPAQUE=1` so the
  overlay branch set an opaque colour instead of `TRANSPARENT_BACKGROUND` still measured a 22 px
  band at 100 % opacity, so the transparent *value* is not what breaks the paint.
- **The layout and the CSS.** A probe injected into the *real* running page reported
  `panel: { pos: "fixed", b: {x:0, y:0, w:304, h:480} }`, `ol: { b: {y:82, h:365} }`, 56 `<li>`
  items, `html`/`body` at full height. The DOM is exactly what it should be.
- **Device pixel ratio.** The real page runs at `devicePixelRatio` 1.25; the harness reproduces at
  1.0 and 2.0 alike.
- **Element promotion hints.** `will-change: transform`, `transform: translateZ(0)`,
  `backface-visibility`, `contain: paint`, `isolation`, a steady `opacity: 0.999`, a CSS animation,
  `backdrop-filter`, and an opaque root background were each measured: **all still 9.1–12.6 %**
  after eight ticks.
- **Forcing damage from the page.** Adding/removing a full-viewport element (`display` toggle), a
  width reflow, replacing `document.body`, and painting a full-viewport `rgba(0,0,0,0.004)` layer
  were each measured: **all still 12.6 %**.
- **Forcing damage from GTK.** `win.queue_draw()` and `view.queue_draw()` per tick: **12.6 %**.
- **WebKit settings.** `hardware-acceleration-policy = 0` (always/never), and
  `WEBKIT_FORCE_COMPOSITING_MODE=1` / `WEBKIT_DISABLE_COMPOSITING_MODE=1`: **12.6 %**.
- **`eval`-based diagnostics.** The app's CSP is `default-src 'self'`, so `window.eval` and
  `fetch` to a local collector are both blocked and report nothing. A probe must be served as
  part of the document (same-origin `<script>`), which is how the DOM facts above were obtained.

## What the mechanism is

Reproduced in a bare WebKitGTK 2.54 / `WebKit2 4.1` harness (`/tmp/fixhunt.py`) with the real built
guide page, mutating one text node per second the way the clock does:

| view background | after 6 ticks |
| --- | --- |
| transparent from creation | **12.6 %** |
| opaque | **100.0 %** |
| transparent set at runtime (created opaque) | **100.0 %** |

So a web view that is transparent **from creation** discards its backing store on every incremental
repaint and redraws only the damaged rectangle. This app is on that path permanently: `wry` 0.57.0
sets the view transparent at creation whenever `attributes.transparent` is set
(`wry-0.57.0/src/webkitgtk/mod.rs:293-295`), which `"transparent": true` in `tauri.conf.json` does
for every launch. The GPU path that might not have this behaviour is unavailable — it fails here
with `Failed to create GBM buffer of size 380x600: Invalid argument`, which is why ADR 0005 forces
`WEBKIT_DISABLE_DMABUF_RENDERER=1` onto the app.

The clock is what makes it visible. Outside a match nothing repaints, so the buffer survives
indefinitely and the guide looks perfect; inside a match it is thrown away once per second and only
the ticking line is redrawn.

## Decision

`BuildOrderGuide.svelte` alternates the root element's opacity between `1` and `0.999` whenever the
clock changes. That forces WebKit to composite the entire layer instead of only the damaged
rectangle, so the buffer is never discarded.

Measured with the fix in place: **100.0 %** of the window painted after 30 consecutive clock ticks,
and across a mode switch (1280x820 → 380x600). On the installed build during a live match:
`transparent (alpha=0): 0 = 0.0%`, `painted (alpha>0): 228000 = 100.0%`, and the screenshot shows
the title, matchup, status line, all 11 steps with times and names, and the footer.

Two details are load-bearing:

- **The values must alternate.** Assigning the same value twice is not a change and WebKit will not
  recomposite, so a fixed `opacity: 0.999` (measured 9.1 %) is not equivalent.
- **0.999, not something visible.** The difference is 0.1 % and invisible on screen, but it must be
  a real assignment each tick. `opacity: 1` to `opacity: 1` would not trigger it.

Removing `"transparent": true` also fixes the painting — it makes the window depth-24 and the
overlay an opaque rectangle, which is not the product. The palette CSS from ADR 0017 stays: it is
what makes the analysis view opaque, and it is unrelated to this fault.

## The second, independent fault: the flip loses the position

The same report described the overlay "appearing at the centre of the screen", moving to the correct
position for an instant when the hotkey was pressed, then snapping back to the centre.

`platform::set_unmanaged` changes the override-redirect attribute by `hide()` →
`set_override_redirect()` → `show()`, because a `GdkWindow` cannot change the attribute while it
exists. That cycle destroys and recreates the `GdkWindow`, so the toolkit and the window manager
place the new one from scratch. Nothing carries the old position across — and the window manager's
default for a fresh unmanaged window is the centre of the screen. Measured: an overlay anchored at
`(24, 24)` reappeared at `(1730, 754)`, which is precisely the centre of a 3840x2160 screen for a
380x600 window. Because the re-show lands *after* the caller's move request, the visible result is
the "moves correctly, then snaps back" the user described.

The fix re-asserts the anchor immediately after each flip, in both directions: `claim_unmanaged`
calls `reposition_for_overlay(window, None)` after `set_unmanaged(.., true)`, and
`release_unmanaged_and_visibility` calls it after `set_unmanaged_and_visibility(.., false, ..)`
when visibility is to be restored. The game rectangle is looked up again rather than threaded
through, so either path is correct regardless of how the caller found it.

Verified on a live match: with the game active and fullscreen, the window became override-redirect
(`ovr=1`) and stayed at `(24, 24)` across 14 samples over 10 seconds, having previously jumped to the
screen centre on exactly that transition.

## Consequences

- The guide stays fully painted during a match. The cost is one style assignment per clock tick,
  i.e. once a second while a game is being played, and none otherwise.
- The fix is in the view, not the shell, because the fault is in the view's compositing. Any future
  guide-like transparent view must do the same.
- Two independent faults were found in the same window and must not be confused: the painting one
  is in the view, and the centring one is in the shell. No CSS change will move the window and no
  placement change will repaint it.
- **An override-redirect flip loses everything the window manager does not store on the window.**
  Position and click-through have both been measured to disappear across it. Anything else that
  lives in window-manager state rather than in the app must be re-asserted after `set_unmanaged`,
  in both directions.
- A transparent web view is now a known-brittle part of the stack on the software renderer. If the
  overlay is ever moved to its own window (the pre-rewrite design), that window must not be created
  transparent and then rely on nothing being repainted.

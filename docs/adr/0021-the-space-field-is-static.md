# 21. The Space field is static, because a software-rendered animation costs a core

**Status:** accepted

**Date:** 2026-10-19

**Supersedes:** the animated background introduced with the themes in ADR 0020.

## Context

The Space theme shipped with three CSS animations: two starfield layers drifting at different
speeds, and a nebula layer breathing. All three were pure CSS — repeating radial gradients and
large blurred radial gradients, no canvas, no images — and slow, at 90 to 520 seconds per cycle.

The user reported that the Space theme made the whole app laggy (clicking, resizing, button
presses) and put a large load on the machine, and that the other three themes were fine.

## Measurements

Taken on the release build, on the user's machine, in **steady state**. The settle time is
load-bearing and cost an hour: the first ~20 seconds after launch are dominated by page load and
rasterisation, and a reading taken at 10 s showed 35.7% for a build that measured 3.4% at 30 s.
Two earlier rounds of bisection compared a warm-up against a steady state and drew exactly the
wrong conclusion, which is why every number below was taken 28 s after launch.

Measured as CPU of the app process *and* its `WebKitWebProcess` child. Measuring only the app
process shows ~1% in every case and hides the fault completely — the page renders in the child.

| Build | Web-process CPU |
| --- | --- |
| Terran / Zerg / Protoss | 0.3 – 1.0 % |
| **Space as shipped** | **100 %** |
| Space, all three animations removed | 4 % |
| Space, layers animated but stepped to ~1 repaint / 15 s | 5.8 % |
| Space, layers animated stepped to ~1 repaint / s | 17.5 % |
| Space, six 4x4 px dots animating at 60 fps | 100 % |
| Space, animations removed but `filter: blur(40px)` removed too | 4 % |

### The cost is per composite, not per pixel

The 4x4 px result is the important one. Six five-pixel dots animating at 60 fps cost exactly as
much as two full-window gradient layers animating at 60 fps. Stepping those full-window layers
down to one repaint per 15 seconds brought them to 5.8%, and one per second gave 17.5%.

So what is expensive is *a composite happening at all*, not the area it covers. That is not how a
compositor is supposed to behave, and it is the signature of a software renderer repainting the
whole window on the CPU for every frame — one where damage rectangles and layer promotion, the
mechanisms that make a GPU-composited animation cheap, are not available.

### It is not the transparency fault from ADR 0019

The obvious suspect was the transparent-webview damage bug already recorded there, since it also
produces whole-window repaints. Setting `"transparent": false` and re-measuring the **unchanged**
animated theme still gave 100.8%. A different fault in the same subsystem, or no fault at all —
just the expected cost of that path.

### Why the lag was specific to Space

`nudgeRepaint` (ADR 0019) assigns an opacity on the root element on every pointer event, to repair
the transparent view's discarded backing store. That forces a full-window composite per event.
With a static page each composite is cheap — measured 2.7% of a core at 241 pointer events a
second — and with an animated Space background each one is ruinous, so the UI thread saturates and
every interaction lags. The lag and the background load are one cause, not two.

## Decision

The Space ambient field is **static**. The starfield and the nebulae stay; nothing in the theme
animates.

The `filter: blur(40px)` on the nebula layer is kept. It is what makes three plain radial gradients
read as soft nebulae, and a *static* blurred layer is rasterised once and then left alone — the
cost was only in re-rasterising it every frame.

`@media (prefers-reduced-motion: reduce)` no longer has anything in the theme to stop. The global
rule stays for the rest of the app.

## Consequences

- Space costs the same as the other themes (3.7% against 0.3%, the difference being the two extra
  composited layers). The theme-specific lag is gone because its cause is.
- The background is still themed and still present; it simply does not move. The nebula gradient
  and the two starfield layers are unchanged, so the palette and composition are what was
  approved.
- **Do not reintroduce motion here without measuring it**, and measure it in steady state on a
  release build, watching the `WebKitWebProcess` child rather than the app process.
- The real exit is a working GPU compositor. `WEBKIT_DISABLE_DMABUF_RENDERER=1` is mandatory today
  (ADR 0005): the DMABUF path fails with `Failed to create GBM buffer of size 1280x820: Invalid
  argument` and renders nothing at all. On a GPU-composited path, transform and opacity
  animations are the cheap case, and the numbers above would not apply. Two other workarounds in
  this codebase exist only because of that fallback — ADR 0019's `nudgeRepaint` and the styled
  scrollbar from ADR 0017 — so fixing the renderer may make all three unnecessary at once.

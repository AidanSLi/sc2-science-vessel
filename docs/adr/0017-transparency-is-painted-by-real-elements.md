# ADR 0017 — Transparency is painted by real elements, and scrollbars are styled

- Status: accepted
- Date: 2026-10-18
- Deciders: flashandfoul

## Context

The app has one window with two modes. `src-tauri/tauri.conf.json` declares it
`"transparent": true` so overlay mode can sit over the game, and mode switching
re-shapes the same window (ADR 0016, ADR 0005). Commit `d79c0e4` added that flag to
fix the guide painting a solid rectangle over the game.

Two things then broke, and the causes are unrelated to each other, which is why
neither could be found by reading the diff.

**The analysis view became a see-through pane.** Under a transparent webview, WebKitGTK
does not paint a `background-color` set on `html` or `body`. Measured on the window's
own ARGB buffer with `XGetImage` (which no other window can occlude, unlike a
screenshot): 1,032,380 of 1,049,600 pixels — 98.4% — had alpha 0, while the page's text
and cards drew normally, so the desktop showed through the window. The same stylesheet
in a non-transparent window covered 94.5% of the region, which isolated the fault to
the root background rather than to the CSS. `transparent` is a creation-time-only
option in Tauri 2.12.3, so it cannot be toggled per mode.

**The overlay's build order steps vanished.** The rows were in the DOM the whole time
(the page's own layout reported the guide `<ol>` at full height), but almost none of
them reached the window. The default WebKitGTK scrollbar is themed by GTK, which makes
it the only part of the page WebKit hands to a separate compositing layer under the CPU
path (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, ADR 0005). Everything outside that layer was
dropped: a `position: fixed; inset: 0` panel wrapping an `overflow-y: auto` list filled
5.5% of a 380x600 window and left 94.5% at alpha 0 — matching the 4.2% measured on the
real overlay, a 21px-wide, 457px-tall strip at the right edge.

A third report — the hotkey revealing the overlay instead of analysis outside a match —
was not a bug in the mode rule. Two GUI processes were running, because
`single_instance.rs` is wired into the watcher binary but not the GUI: a process started
before the reinstall had survived it and was still holding `SC2SV_PIN_OVERLAY=1`, which
short-circuits every mode decision to `Overlay`. The rule itself was verified correct
against the live client API (`/ui` reporting the four menu screens ⇒ `Analysis`).

## Decision

**Paint through real elements, never through the document roots.** A transparent-ARGB
window needs a full-viewport block-element surface, positioned so it cannot collapse:

- `.app-surface` (`position: fixed; inset: 0; background: #0a0d12`) is the analysis
  view's base, added by `src/routes/+layout.svelte`. It must stay `position: fixed`;
  a flow-layout `height: 100%` div measured 98.4% alpha-0, and switching it to
  `fixed; inset: 0` took the same build to 100% painted with zero other changes.
- `.guide-panel` (`position: fixed; inset: 0`) is the overlay's, and at
  `--guide-veil: 0%` it mixes to a 0.4% black veil rather than `transparent`. A page
  whose roots are transparent and whose content is text-only paints *nothing at all*
  (0 of 14,362 expected text pixels); any non-zero alpha on one full-size element
  brought 7,393 of them back. One part in 255 is below the threshold of visibility
  while still giving WebKitGTK a surface to composite onto.

**Style the guide's scrollbar.** `.guide-steps::-webkit-scrollbar` sets a 6px width and a
veil-matched thumb. Any custom scrollbar style restores painting — `width: 6px`,
`scrollbar-width: none` and `display: none` each measured 100% — so the bar is kept and
made narrower rather than removed, because a long build order still needs to show that
it scrolls. `overflow: hidden` also painted 100% but truncates the list, which is the
entire content of the overlay.

**Set the veil's default at document level.** `html.is-transparent-window { --guide-veil:
0% }` exists so the `color-mix()` expressions are never invalid at computed-value time.
Before this, `--guide-veil` was documented as "set from the component's `panelOpacity`"
and no code ever set it, so both declarations were dropped and the guide silently lost
its rules and row surfaces. The component now sets it inline from `panelOpacity`.

## Consequences

Both views are opaque or see-through by construction rather than by inheritance, so
mode switching no longer depends on which background happened to be painted.

The cost is that page CSS must keep carrying a full-viewport element for each mode. A
new view that renders only text and relies on the body's background will be invisible
in overlay mode, and a new scroll container inside the guide will need the same
`::-webkit-scrollbar` treatment or its contents will vanish. Both failure modes are
silent — the page reports correct layout while the pixels are absent — so verification
has to read the window's own alpha (`XGetImage`), not take a screenshot and look.

Not done: `single_instance.rs` is still only used by the watcher binary, so two GUIs can
coexist and reintroduce the pinned-overlay confusion. That is worth closing separately.

Two dead ends are recorded so they are not retried: `set_background_color` on the
`WebviewWindow` also repaints the GTK toplevel through a generated CSS rule
(`tao-0.37.1/src/platform_impl/linux/event_loop.rs:406-419`) and put the colour *in
front* of the page, producing a fully white window; and `WEBKIT_FORCE_COMPOSITING_MODE`,
`WEBKIT_DISABLE_COMPOSITING_MODE` and re-enabling the DMABUF renderer all left the
overlay at 4.2% (the DMABUF path also fails here with `Failed to create GBM buffer`).
Page CSS alone fixes both bugs, so no Rust window code is needed.

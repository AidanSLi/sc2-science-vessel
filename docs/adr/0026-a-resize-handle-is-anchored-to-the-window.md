# 0026 — A resize handle is anchored to the window, not to the page

- Status: accepted
- Date: 2026-10-21
- Supersedes: nothing; amends ADR 0025 (the sticky title bar) and ADR 0020 (the chrome is page
  markup)

## Context

The window is frameless in both modes, so it has no toolkit title bar and no toolkit resize
border. Both are ordinary page markup (ADR 0020). The eight resize handles lived inside
`src/lib/components/TitleBar.svelte` as `absolute` children of its `sticky` header.

Two independent things were wrong with that, and only one of them was visible.

The `South` handle was `absolute bottom-0`, which resolves against the **title bar's** bottom
edge, about 36 px below the top of the window. Its `onpointerdown` called
`startResizeDragging('South')`, which names a side of the **window**. So the strip along the
bottom of the title bar resized the bottom of the window: the pointer never went near the edge
being dragged. Reported as "the bottom of the title bar is draggable, and doing so resizes the
bottom of the window".

`North`, `West` and `East` were wrong in the same way but coincidentally correct in effect,
because the bar spans the window's full width and its left/right edges are the window's. Only
`North` was meaningfully off (the bar's top edge was the window's top edge too, since the bar is
the first child). That accident is why the fault looked like a single broken side rather than
eight.

The handles also travelled with the bar once it became `sticky` (ADR 0025), so on a scrolled
page all eight were dragged along with it while the window edges stayed put. That made the
latent geometry bug much easier to hit.

## Decision

Handles are anchored to the **viewport**. For a frameless window the viewport *is* the window, so
a `position: fixed` box at `bottom: 0` is at the window's bottom edge and `'South'` is the
direction that moves it. Box and direction are then measured against the same thing, which is the
whole invariant.

They live in a new `src/lib/components/WindowEdges.svelte`, rendered by
`src/routes/+layout.svelte` in the analysis branch only, from a table in
`src/lib/window-edges.ts`:

| placement | box | direction |
| --- | --- | --- |
| `north` | `top: 0`, inset `1rem`, `height: 4px` | `North` |
| `south` | `bottom: 0`, inset `1rem`, `height: 4px` | `South` |
| `west` | `left: 0`, inset `1rem`, `width: 4px` | `West` |
| `east` | `right: 0`, inset `1rem`, `width: 4px` | `East` |
| `north-west` / `north-east` / `south-west` / `south-east` | `16px` square in the corner | the diagonal |

Four consequences, each load-bearing:

1. **No handle inside the sticky title bar.** The bar scrolls with the page's flow; a window edge
   does not. Anything positioned inside the bar is measured against the bar.
2. **The placement rules are written out, eight of them.** A composed version
   (`[data-placement^='north']`, `[data-placement$='west']`) reads better and is wrong:
   `[data-placement^='north']` also matches `north-west`, so `left: 0` for the west edge and
   `left: 1rem` for the north-west corner would fight over one property and source order would
   decide the winner. Eight explicit blocks cannot collide.
3. **Corners and edges are inset by the same `1rem` so they interlock instead of overlapping.**
   A corner owns its 16×16 square; every edge starts just past it. Overlapping boxes would make
   the element under the pointer depend on paint order rather than on where the pointer is.
4. **Overlay mode gets no resize border.** It is a 380×600 override-redirect panel the app
   positions itself (ADR 0016); growing a resize border there would let the player pull it out of
   the anchor the watcher keeps re-applying.

There is deliberately no separate drag strip along the top edge. The title bar is directly under
it and already spans the full width as the move surface, so a second strip would only be
somewhere for the north edge and the north corners to collide with.

## Consequences

`src/lib/window-edges.test.ts` solves all eight rectangles from `EDGE_THICKNESS` and
`CORNER_SIZE` and asserts: eight unique directions and placements; no two boxes intersect; every
box has positive area and stays inside the viewport; every point along all four window boundaries
is covered by some handle (which is what catches an inset that is not mirrored); and the
stylesheet carries the numbers the solver uses, so CSS and test cannot drift apart. It also
asserts `TitleBar.svelte` contains no resize call and no handle class, so the handles cannot
migrate back into the bar.

Verified against the installed build by hit-testing with `document.elementFromPoint` in a bare
WebKitGTK harness and by dragging the real window with XTest:

| drag point | result |
| --- | --- |
| title bar's bottom edge | `origin unchanged, size unchanged` |
| inside the title bar | `1506,307` → `1981,488` (moved) |
| window's real bottom edge | `989` → `1313` tall (resized) |

The hit test returns `placement=south` at the window's bottom edge and `placement=west` /
`east` / `north` on the other three sides, `north-west` / `south-east` in the corners, and a plain
`HEADER` for the title bar's bottom edge — i.e. ordinary page area, resizing nothing.

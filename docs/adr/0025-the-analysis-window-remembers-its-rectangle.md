# 0025 — The analysis window remembers its rectangle, and the title bar is sticky

- **Status:** accepted
- **Date:** 2026-10-25
- **Supersedes:** nothing. Amends 0020 (the chrome is page markup) by making that chrome
  stationary instead of merely present.

## Context

Two complaints, one shape: the window did not behave like a window.

1. **The title bar scrolled away.** The window is frameless (ADR 0020), so
   `src/lib/components/TitleBar.svelte` is the only place a drag can start, and it also
   carries the eight resize handles. It was a normal flex child of the layout, and the
   scrolling container is `.app-surface` (`src/app.css`, `position: fixed; inset: 0;
   overflow-y: auto`) — not the document. On any page taller than 820 px the player had to
   scroll back to the top before they could move the window. That is the worst possible
   moment to lose a handle: a long page is exactly where a player wants to drag the window
   somewhere else.

2. **The window forgot where it was.** Every show recomputed
   `set_size(1280x820)` and, when revealed, `center()`. So a player who parked the window on
   a second monitor got it back dead centre on the primary one at every hotkey press, and
   every launch. The app is resident and toggled dozens of times per session, so this was not
   a corner case; it was the normal experience of using the overlay hotkey and coming back.

## Decision

### The title bar is `sticky top-0 z-20`

`sticky` positions the bar relative to the *scroll container*, which is `.app-surface`, so
the bar stays at the top of the window while the surface scrolls underneath it. `fixed` would
also pin it but would take it out of the surface's flow, so the layout would have to reserve
space for it by hand and the resize handles would sit at coordinates the layout no longer
knows about. Between the two, `sticky` is the one that leaves the existing flex column
intact: the bar is still a flex item, it just refuses to be scrolled.

Two properties are load-bearing and easy to remove by accident:

- **an opaque background.** The bar is `color-mix(in oklab, var(--color-surface-900) 86%,
  var(--color-accent-2))` over the theme's surface ramp. A translucent sticky header shows
  the scrolling content through it, which reads as a rendering bug and makes the window
  buttons hard to hit.
- **a stacking context above the content and the ambient layers.** `z-20` clears both the
  page content and the `position: fixed` `.theme-sky` / `.theme-nebulae` layers. Without it
  the Space theme's starfield paints over the bar's own background.

The eight resize handles stay inside the header, because a handle must travel with the
surface it resizes. They are still siblings of the `data-tauri-drag-region` element, which is
the constraint from ADR 0020 that a handle inside the drag region starts a move.

### The rectangle is persisted as physical pixels, captured from window events

`src-tauri/src/window_geometry.rs` owns the rectangle. The shape of the decision:

- **Physical pixels, not logical.** A logical rectangle saved while the display was at 2.0
  scale and restored at 1.0 both moves and resizes the window. The only consumer is
  `set_position`/`set_size`, which take physical values, so storing physical removes the
  conversion rather than adding one.

- **Captured from `Moved`/`Resized` window events, not on demand.** The rectangle has to
  survive the moment the window is hidden, and there is no "about to hide" event to hook. A
  drag also emits dozens of `Moved` events, so the capture writes to an in-memory
  `Mutex<Option<WindowGeometry>>` and a 1 Hz debounce thread flushes it to `settings.json`.
  `toggle_window`'s hide path and the visible-start path both flush immediately, so an
  ordinary session never loses the last move to the one-second window.

- **Two one-shot match guards, as `AtomicBool` flags.** `RESTORING` swallows the
  `Moved`/`Resized` events that the restore itself causes, so a restore is not immediately
  recorded as if it were a player action. `PLACED` is the more interesting one: until
  `restore` has run, every `Moved`/`Resized` event describes the window's *initial* placement
  from `tauri.conf.json` — 1280x820 centred — and not anything the player chose.

  `PLACED` exists because of a measured regression, not a theory. With `RESTORING` alone, a
  window saved at `1100x700+300+150` came back at `1280x820+1280+644` one launch later: the
  start-up resize was captured as if it were a drag, and the next flush wrote it over the
  saved value. The ratchet is invisible in review — every individual write is plausible — so
  the guard needs to stay.

- **The restore happens after `show()`.** A geometry request against an unrealised
  `GdkWindow` is lost. Every call site (`apply_window_mode`'s analysis branch,
  `warm_or_reveal_main_window`) therefore restores after the window is mapped, and the
  restore re-reads the in-memory pending value before falling back to the file, so a move that
  has not been flushed yet is not undone by the next reveal.

- **Only analysis mode records anything.** The overlay is deliberately repositioned and
  resized to 380x600 on every entry (ADR 0016/0017), so recording its rectangle would
  overwrite the player's choice once per match. A maximised or fullscreen window is excluded
  for the same reason it is easy to miss: it reports the *screen's* rectangle, and no
  containment test can tell that apart from a player who really did fill the screen.

- **A saved rectangle is only restored if it is still reachable.**
  `WindowGeometry::is_restorable` requires at least 64 px of overlap on both axes with some
  currently attached monitor, and a size at or above `ANALYSIS_MIN_WIDTH`/`ANALYSIS_MIN_HEIGHT`.
  Overlap rather than containment is deliberate: a window parked half off-screen is a
  perfectly good place to work, while a window that merely grazes an edge is a lost window.
  When the test fails, the window falls back to 1280x820 centred — the old behaviour, which
  is the right answer for a monitor that has been unplugged.

- **`Option<WindowGeometry>`, not a defaulted rectangle.** "Never placed" and "placed at the
  default spot" have to be distinguishable, or a first run would restore a position the app
  invented and a monitor change would be indistinguishable from a player choice.

## Consequences

- The window now behaves like a window: the bar is always draggable, and the rectangle
  survives hide/show and restarts.
- `settings.json` gained a nested `analysisGeometry` object, so the file is no longer a flat
  map of scalars. It is hand-editable like the rest of the file, and `is_restorable` is what
  makes a hand-edited or stale value harmless rather than a lost window.
- The asymmetry between the two reveal paths is now a known hazard: a new place that shows the
  window must call `window_geometry::restore` (or go through `apply_window_mode`), or it will
  silently ignore the saved rectangle. This was the first of the two bugs above.
- `flush` skips the write when the value is unchanged, so an idle session does not rewrite
  `settings.json` once a second and does not invalidate any file watcher.
- Verified on the installed release build: saved `{300,150,1100x700}` restored as
  `1100x700+300+150`; dragged to `{400,200,1150x740}` and relaunched at `1150x740+400+200`;
  then `1180x760+700+200` → hotkey hide → hotkey show → still `1180x760+700+200` in the same
  session; and `1024x660+640+300` survived a relaunch. The sticky bar was checked at
  `scrollTop = 400` with the Milestones card tucked beneath it.
- Measurement trap worth recording: while verifying, the app window read `map_state=Unmapped`
  and every screenshot was black, with no app window found at all. The cause was **DPMS
  sleep** — `kscreen-doctor --dpms on` fixed it and the window instantly became `Viewable`.
  This is the second time in this project that a sleeping display was mistaken for an app
  fault; check `kscreen-doctor -o` before believing an unmapped or black window.

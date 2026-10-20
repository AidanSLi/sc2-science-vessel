# ADR 0020 — Themes are CSS variable overrides, and the window chrome is page markup

- Status: accepted
- Date: 2026-10-19
- Deciders: Aidan

## Context

The app had a single palette and a native title bar. Two requests came together:

1. Offer race-themed appearances (Terran, Zerg, Protoss) plus a "Space" theme, and let the user pick
   one in Settings.
2. Make the title bar flush with the app's own background, i.e. not a toolkit-drawn strip in the
   desktop's colours sitting above a themed panel.

Both are cheap or expensive depending on where the styling lives.

Three constraints from earlier work decide that:

- **Tailwind 4 has no `tailwind.config.js`.** Its theme *is* the `@theme` block in
  `src/app.css:18-53`, so every colour utility in the markup (`bg-surface-900`, `text-ink-300`, …)
  is generated from those custom properties.
- **ADR 0019: a transparent WebKitGTK view, transparent from creation, drops its backing store on
  every incremental repaint.** A theme change that repainted the whole tree from JavaScript would
  be exactly that kind of large in-place redraw under a transparent view — the path measured at
  12.6% painted after six ticks and 100% only when a full-layer composite is forced. The overlay is
  a transparent window; theme switching must not fight that.
- **The window is a single window in two modes** (ADR 0016). `apply_window_mode` owns decorations,
  and the overlay mode already runs with `decorations: false`.

## Decision

**A theme is a set of custom-property overrides keyed on `html[data-theme]`, and the window chrome
is markup in the page rather than a toolkit decoration.**

Specifics:

- The palette lives in `src/app.css` as `@theme` (Terran, i.e. the default) followed by
  `html[data-theme="zerg|protoss|space"]` blocks that override the same variables. `html[data-theme]`
  outranks `:root`, so a theme switch is one attribute write and no component re-render at all —
  which is why it does not disturb ADR 0019's repaint path.
- Semantic colours (`--color-ahead`, `--color-behind`, `--color-neutral`, `--color-race-*`) are
  **deliberately not themed**. Green means "ahead of the build" in every race's console; a skin that
  recoloured it would change what the guide's most load-bearing signal means per theme.
- Ambient layers (`--theme-sky`, `--theme-nebulae`) render only for Space and only in the analysis
  view, as pure CSS gradients behind the content. They are `pointer-events: none` and never appear in
  the overlay, where stars behind a build order are unreadable noise.
- The theme is persisted in `settings.json` (Rust), not `localStorage`, and read **before** the first
  paint: `src/app.html` ships `data-theme="terran"` in the markup and `initTheme()` replaces it from
  the shell. A theme applied after hydration would flash the wrong palette every launch.
- The window is declared `decorations: false` and the title bar is `TitleBar.svelte`: a
  `data-tauri-drag-region` strip, three window buttons, and eight absolutely-positioned resize
  handles calling `startResizeDragging`. The resize handles are siblings of the drag region, because
  a handle inside a drag region starts a *move* instead of a resize.
- `apply_window_mode` no longer re-enables decorations in the analysis branch. Decorations are off in
  both modes; only the analysis layout mounts the title bar.

## Consequences

Easier:

- A new theme is one CSS block plus one entry in `THEMES` (`src/lib/theme.ts`). No component knows
  which theme is active.
- The title bar follows the theme exactly, because it is page markup.
- Switching themes costs one attribute write and paints nothing.

Harder, and now committed to:

- **A frameless window has no toolkit resize border, so the page must provide one forever.** If
  `TitleBar.svelte` is ever removed from the analysis layout the window becomes immovable and
  unresizable. `capabilities/default.json` must keep `core:window:allow-start-resize-dragging`,
  `allow-start-dragging`, `allow-minimize`, `allow-toggle-maximize`, `allow-is-maximized` and
  `allow-close`.
- Accessibility of the drag region has to be argued, not assumed: `role="presentation"` is used
  because a move handle is not a keyboard-operable control, and its double-click-to-maximise
  duplicate is the reachable maximise button.
- The theme id crosses two untrusted boundaries (a hand-edited settings file and an IPC reply), so
  both sides validate it: `parseTheme()` in the frontend and `settings::THEMES` in Rust. An unknown
  id must never reach `data-theme`, or every theme-scoped rule stops matching.
- Only `terran` is exercised by the default test suite; the other palettes are checked by eye. A
  contrast regression in `zerg` or `protoss` would not fail a test.

If the assumption behind this is wrong — that is, if a future toolkit can style its own headerbar
from page CSS — the frameless chrome is unnecessary and could be replaced by a native title bar.
Nothing else depends on it.

# 8. Run the overlay on the X11 backend, and anchor it to the game window

- **Status:** Accepted
- **Date:** 2026-10-15
- **Deciders:** maintainer
- **Supersedes:** the "ship always-on-top toplevel first" staging in ADR 0003, and the
  Wayland-only `set_always_on_top` assumption behind ADR 0004's overlay notes.

## Context

Four things were reported broken on Linux/KDE/Wayland, all of them with the same root cause:

1. The overlay stops being on top as soon as the game window has focus.
2. The overlay cannot be positioned; it appears at a fixed screen coordinate.
3. It should stay above the game but be allowed to fall behind unrelated focused windows.
4. The global hotkey does nothing while the game has focus.

The cause is structural, not a misconfiguration. On Wayland a client cannot do any of these
three things, and KWin offers no workaround:

- **`set_always_on_top` is a no-op.** GTK3's `gdk_wayland_window_set_keep_above` is an empty
  function. Its only Wayland-side lever is `xdg_activation_v1`, which raises a window once and
  then lets the compositor decide. Confirmed by inspection of a KWin capture: the window
  reports `keepAbove=true` at the same moment it is behind a fullscreen game, and its requested
  layer is 3 (AboveLayer) rather than 9 (OverlayLayer), which is the band above fullscreen.
- **`set_position` is a no-op.** There is no protocol for a client to place its own toplevel.
- **A Wayland client cannot read another application's geometry**, so even with a position it
  could not anchor to the game.
- The plugin route for the hotkey is X11-only (`XGrabKey`), so it never sees a keypress while
  another window holds focus. That part is fixed separately by ADR 0007's portal work.

The `wlr-layer-shell` protocol would give a true overlay surface, and KWin implements it, but
it requires libwayland/`wlr-layer-shell` bindings and gives up the WebView's ordinary window
behaviour. That remains the eventual target (ADR 0003); this ADR is the shipping mechanism.

## Decision

**Force GTK onto the X11 backend through XWayland when an X server is present, and use X11
to locate the game window.**

`main.rs::prefer_x11_when_xwayland_is_present` sets `GDK_BACKEND=x11` before GTK initialises,
unless the user has set `GDK_BACKEND` themselves (an explicit choice always wins) or no X
server is reachable (where forcing X11 would prevent start-up). The reasoning is that
StarCraft II under Wine is itself an XWayland window, so X11 is the one protocol on which both
the game and the overlay are addressable at the same time.

On the X11 backend GTK honours all three missing properties, verified against KWin on the
reference machine (3840x2160, 200% scaling):

| Property | Wayland backend | X11 backend |
| --- | --- | --- |
| `keepAbove` | `true` but ignored | `true` and honoured |
| KWin layer | 3 (AboveLayer) | 3 (AboveLayer), above the game in practice |
| Position | request dropped, lands at a default | honoured |
| Transparency | works | works (RGBA visual available) |

`x11_overlay.rs` then does the anchoring: walk the window tree, match windows whose
`_NET_WM_NAME`/`WM_NAME` begins with `starcraft ii` or `starcraft 2`, keep the largest
*viewable* one, and translate its geometry into root coordinates with
`translate_coordinates` — `get_geometry` alone is parent-relative, so an overlay placed from it
would drift by the window frame's offset the moment the game is moved.

The overlay is placed at the game's top-left corner plus a 24 logical-pixel inset, converting
from X11's physical pixels using the window's scale factor, because a hard-coded pixel offset
would be twice the intended distance at 200% scaling.

## Consequences

- **The overlay is now on top of the game and follows it**, which was the whole point.
- **Transparency and click-through keep working.** Both were already verified.
- **The app runs through XWayland on a Wayland session.** This is a real cost: XWayland adds a
  compositing hop, and the app does not participate in native Wayland scaling or input
  methods. For a WebView capped at ~1 repaint per second while following a game clock, this is
  not measurable.
- **On a Wayland session without XWayland the app stays on Wayland** and prints why, rather
  than failing to start. The overlay's keep-above and anchoring are unavailable there; the
  honest-failure posture of the rest of the app applies.
- **The game window name is a heuristic.** Wine reports the launcher's `WM_CLASS`
  (`steam_app_default`), so the title is matched instead, and the largest viewable match wins
  because Steam and Battle.net keep extra toplevels whose titles start with the game's name.
  A localised or renamed window would not be found; that degrades to the fixed fallback
  position, not to a failure.
- A game window that is minimised, on another virtual desktop, or hidden reports
  `MapState::UNMAPPED` and is correctly skipped, so the overlay does not anchor to a stale
  rectangle.

## Still open

- **Fullscreen.** Anchoring and stacking are verified with the game windowed. A truly exclusive
  fullscreen game may bypass the compositor entirely, and the correct fix there is the
  `wlr-layer-shell` overlay surface from ADR 0003 rather than more X11 tuning.
- **Following a moving game window continuously.** The position is recomputed on each toggle
  and not on a timer. If the player moves the game while the overlay is up, it stays put until
  the next toggle.
- **Multi-monitor.** The position is taken from the game's own rectangle, so it follows the
  game across monitors, but this has only been exercised on the reference machine's layout.

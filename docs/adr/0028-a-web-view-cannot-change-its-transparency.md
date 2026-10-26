# 0028 — A web view cannot change its transparency after its first paint

- Status: accepted
- Date: 2026-10-26
- Supersedes in part: ADR 0019 (which described the same WebKitGTK fault but concluded that a
  forced re-composite was a sufficient repair)
- Related: ADR 0005 (the software renderer), ADR 0016 (one window, two modes), ADR 0027 (the title
  bar sits above the scrolling body)

## Context

The user reported that **hovering a button, a title-bar control, or picking an option from the theme
dropdown made the whole window except the hovered element disappear**. That is the symptom ADR 0019
records, reached through a path ADR 0019's fix does not cover.

ADR 0019's fix (`nudgeRepaint`, and the clock-driven opacity flip in `BuildOrderGuide.svelte`) forces
a full-layer composite *after* the damage. It keeps the window painted while something is already
ticking, which is why the overlay looked fixed, but it cannot prevent the blank frame itself: the
damage is applied, the backing store is discarded, and only the damaged rectangle is redrawn. On a
window with no clock running — the analysis view — nothing re-composites, so the blank state stays.

Measuring this took several wrong turns worth recording, because each produced confident nonsense:

| instrument | why it lied |
| --- | --- |
| alpha coverage of the window buffer | the analysis window is **fully opaque** (alpha 255), so a "disappeared" window reads as 100% painted |
| `ffmpeg -f x11grab` | returns pure black under XWayland compositing (`((0,0),(0,0),(0,4))`) while `spectacle` returns real content at the same instant |
| `spectacle` screenshots | ~0.48 s per capture, so a 1–2 frame flash is never in the picture |
| `XWarpPointer` hover trials | a warp is applied server-side by KWin/XWayland and produces no enter/leave motion the toolkit acts on; the Close button's pixels were **byte-identical** (0 of 1080) with the pointer on it, and a synthetic click did not close the app |

The instrument that works is the **distinct-colour count** of the window's own buffer: a blank window
has one colour, a painted one has hundreds. Reproduced in a bare WebKitGTK harness
(`/tmp/hovrep.py` + `/tmp/static.html`) by toggling one button's background — the exact damage a
`:hover` causes — then settling and counting.

## The measurements

Where the opaque background colour is set, and what that does to ten consecutive hover-like restyles:

| opaque colour applied | distinct colours per trial | verdict |
| --- | --- | --- |
| after the page has finished loading (today) | `[431, 1, 1, 1, 1, 42, 87, 129, 154, 152]` | **BLANKED** |
| after the window is shown, before the page loads | `[431, 430, 429, 431, 420, …]` | fine |
| before the window is shown | `[431, 430, 429, 431, 420, …]` | fine |
| view created opaque, no colour ever set | `[344, 331, 332, 335, 331, …]` | fine |

So the load-bearing event is **the page's first paint while the view is transparent**. That is when
WebKit decides it never needs a full surface. Once that has happened, nothing recovers it:

| attempted repair after the blanking state is set | result |
| --- | --- |
| `set_background_color` with an opaque colour, again | still blanks |
| a full-surface resize | still blanks |
| `reload()` under an opaque background | still blanks |
| root-opacity alternation (`nudgeRepaint`) | blanks first, then recovers *later* — a patch, not a cure |
| flipping to transparent, then reloading | alpha-0 stays `0.0%`, i.e. never becomes see-through |

And a view that has been given an opaque background can **never** be made see-through again: alpha-0
stays at `0.0%` for every flip attempt, while a control view left transparent from creation meaures
`95.2%`.

`wry` 0.57.0 `src/webkitgtk/mod.rs:293-295` only reads the `transparent` flag when it **creates** the
view, applying `set_background_color(RGBA(0,0,0,0))`; `"transparent": true` at
`src-tauri/tauri.conf.json:27` therefore makes every view transparent at creation on every launch.

## Decision

**Two windows, one view each.** Each mode gets a view whose transparency is decided at creation and
never changed afterwards:

- the **analysis** window (`MAIN_WINDOW_LABEL = "main"`) is declared `"transparent": false` in
  `src-tauri/tauri.conf.json`, so its first paint is opaque and no hover, click or popup can blank
  it;
- the **overlay** window (`OVERLAY_WINDOW_LABEL = "overlay"`) is declared `"transparent": true`, so
  the game shows through its text-only page.

Only one is visible at a time. `apply_window_mode` hides the other window first, then applies this
mode's flags, which keeps ADR 0016's central property — one mode for the app, decided in one place —
without its now-impossible implementation.

### Why two windows and not two views in one

Both routes were built and measured. Two views inside one window is measurably equivalent for
transparency (`/tmp/twoview.py`, one `Gtk.Box` holding two `WebKit2.WebView`s):

| state | alpha-0 | distinct colours |
| --- | --- | --- |
| analysis view shown | 0.0% | 256 |
| overlay view shown | **95.2%** | 227 |
| back to the analysis view | 0.0% | 256 |

It was rejected for two reasons. Tauri gates multi-webview behind the `unstable` cargo feature
(`tauri-2.12.3/src/window/mod.rs:1226`) and documents a WebView2 deadlock on Windows under it — and
most of this app's users are on Windows 11. Two windows need no unstable feature, and the split is
what the mode concept already was: two mutually exclusive presentations.

### What each window renders

Both windows load the same page. Each decides its view from its own Tauri **label**
(`viewForLabel` in `src/lib/window-view.ts`), which is the identifier the shell uses to find it, so
the two cannot disagree. Only the analysis window runs the client-API poll and decides the mode: a
second poller asks the shell to switch modes too, and the two undid each other's hides (measured:
both windows mapped at once).

`src-tauri/capabilities/default.json` lists **both** labels. Listing only `main` left the overlay
with no `http:default` access, so its guide displayed "Command plugin:http|fetch not allowed by
ACL" where the status line belongs.

## Consequences

- `set_background_color` is no longer called on a mode change. It is a creation-time property in
  practice, and calling it later is what produced the bug: it looked like it worked, because the
  window *was* opaque, while the view had silently lost the ability to repaint itself.
- The `nudgeRepaint` machinery (ADR 0019) stays, but its role narrows to the overlay view, where a
  transparent view still drops its backing store on incremental repaints. It is no longer load-
  bearing for the analysis window and is no longer the defence against the hover flash.
- `apply_window_mode` owns both windows' visibility and flags, and the existing mode-change path is
  unchanged from the caller's point of view.
- Showing "the app" now means showing the window the current mode belongs to: `show_main_window`
  returns early unless the mode is Analysis, and the visible-start path routes through
  `apply_window_mode(app, window_mode(), Visibility::Shown)`. Showing the analysis window
  unconditionally left both windows mapped at startup (measured), because a start that had already
  switched to overlay mode was revealed again underneath the guide.
- Anything that later needs the analysis window to be see-through must create a third window. It
  cannot reuse the analysis one; that is the whole point of this ADR.
- `set_background_color` is no longer called anywhere on a mode change. Nothing in the frontend can
  fix this class of fault, so a future blanking report should be diagnosed as a creation-time
  transparency question first, not as a repaint-timing one.
- A future WebKitGTK that honours `set_background_color` after the first paint would make this
  arrangement unnecessary, but the creation-time split costs one extra view and no correctness, so
  the simpler invariant is worth keeping until the fault is demonstrably gone upstream.

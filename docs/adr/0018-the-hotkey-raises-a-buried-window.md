# ADR 0018 — The hotkey raises a buried window, and the webview's background decides transparency

- Status: accepted
- Date: 2026-10-18
- Deciders: flashandfoul
- Supersedes: nothing; extends ADR 0016 (mode choice) and ADR 0017 (transparency painting)

## Context

After ADR 0017 fixed the painting faults, three reports remained:

1. With the fullscreen game focused, the overlay hotkey "does nothing".
2. The analysis window's content is "entirely transparent".
3. The hotkey still brings up the build order guide rather than the analysis window when
   the game is running but no match is on.

Report 3 turned out to be two live GUI processes again — a pre-reinstall instance holding
`SC2SV_PIN_OVERLAY=1` — and the mode rule was already correct: with the game in its menus
`/ui` returns four screens, so `hotkey_mode` answers `Analysis`. Verified against the live
client and through the real hotkey (`toggle: revealing in mode=Analysis`).

Reports 1 and 2 were real, and each had its own cause.

### Report 1: a mapped window is not a visible window

The toggle asked `window.is_visible()`, which answers only "is the window mapped". A window
sitting *under* a focused fullscreen game is still mapped, so the hotkey read it as shown and
**hid** it. The player pressed the key, the window disappeared from the only place it could
have been seen, and the key looked dead.

The corrected rule treats a window as visible only when it is mapped *and* the window manager
reports it active. It lives in `hotkey_should_hide` in `src-tauri/src/lib.rs` and was written
into the toggle, the hotkey and the show path so all three agree.

Revealing is then not enough either. Tauri's `set_focus` reaches
`gtk_window_present_with_time`, which asks the window manager to activate with EWMH source
indication **1** ("an ordinary application"). Measured with the fullscreen game focused: the
app stayed at stacking index 3 below the game's 4 and the game stayed active. The same request
with source indication **2** ("a direct user action") was honoured at once — the app became
active and moved above the game. That is the honest value for a global hotkey the player
configured for this purpose, and it is what `platform::raise_and_activate` sends.

The raise also has to be ordered after the mode change and posted to the main thread. Every
geometry call in `apply_window_mode` (`show`, `set_size`, `center`) goes through tao's
event-loop channel rather than executing inline, so a raise issued from inside the function is
processed first and then undone by the resize that follows.

Measured on the installed build with a compositor screenshot, sampling the app's own
`#0a0d12` surface over its rectangle:

| state | visible app surface |
| --- | --- |
| game focused, app buried | 4.7% |
| after one hotkey press | 86.1% |
| press again (hide) | 4.0% |
| press again (show) | 83.9% |

### Report 2: `None` means opaque white, not transparent

The overlay must be see-through and the analysis window must not, from one window whose
`transparent: true` flag is fixed at creation. `wry` 0.57.0 applies that flag to the **web
view** as well (`webkitgtk/mod.rs`: `webview.set_background_color(RGBA(0,0,0,0))`), so the
per-mode difference has to come from setting the webview's background at runtime.

`WebviewWindow::set_background_color` is the wrong tool — it also sets the *window* colour,
which tao implements as a generated GTK CSS rule on the toplevel that paints **in front of**
the page. Measured: asking for `(10,13,18)` produced a 98.4% white window and `(0,8,255)`
produced `(0,255,255)`. The webview-only setter (`window.as_ref()` as `&Webview<Wry>`)
round-trips exactly.

And `None` is not "clear it": tauri-runtime-wry 2.12.1 maps `None` to opaque white at
`src/lib.rs:3789` (`color.map(Into::into).unwrap_or((255, 255, 255, 255))`). Passing it for
overlay mode painted the guide onto a 94.3% `rgba(254,254,254,255)` panel that hid the game.
The transparent value is `Some(Color(0, 0, 0, 0))`, now the named constant
`TRANSPARENT_BACKGROUND`.

`apply_window_mode` therefore sets the webview background on **both** branches, not only when
leaving analysis: the startup path applies a stored mode directly without the other having been
left, and a guide that came up with an opaque backdrop would hide the game it floats over.

The analysis half of that is redundant rather than load-bearing: with the setter removed, three
consecutive launches still painted 100% of the window from `.app-surface` alone (ADR 0017). It is
kept as a second defence and because it is the same colour, so the two never disagree on screen.

## Decision

- Visibility for the toggle means mapped **and** active, via `hotkey_should_hide`.
- The hotkey raises with EWMH source indication 2 (`platform::raise_and_activate`), after the
  mode has been applied, on the main thread.
- Per-mode transparency is set on the **webview** only: `TRANSPARENT_BACKGROUND`
  (`Color(0,0,0,0)`) for overlay, `ANALYSIS_BACKGROUND` (`#0a0d12`, matching
  `--color-surface-950`) for analysis. Never `None`, never `WebviewWindow::set_background_color`.
- Both branches set it on every entry, so no path can inherit the other mode's background.

## Consequences

- The hotkey now works the way a player expects: it reveals over a focused fullscreen game and
  toggles normally otherwise.
- Two independent facts about this stack are easy to get wrong and are now written down in
  AGENTS.md: a mapped window is not a visible window, and `set_background_color(None)` is
  opaque white.
- `set_focus` is still called after the raise. It is redundant on X11 but harmless, and it
  keeps non-Linux platforms working, where `raise_and_activate` answers `false`.
- The EWMH request is Linux-only by construction. A window manager that ignores source
  indication 2 would fall back to `set_focus`'s behaviour, i.e. reveal without raising.

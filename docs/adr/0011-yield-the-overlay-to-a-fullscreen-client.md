# 11. The overlay yields the top to a fullscreen client instead of pretending to hold it

- **Status:** Accepted
- **Date:** 2026-10-16
- **Deciders:** maintainer
- **Related:** ADR 0008 (the X11 backend and restacking), ADR 0003 (the layer-shell target that
  is the only real cure)

## Context

The user asked for the overlay to stay over the game, and then to work "when the game is running
in fullscreen". Those two requests turn out to conflict, and the conflict is in the compositor,
not in this app.

KWin decides a window's z-band in `Window::belongsToLayer()` (`src/window.cpp`), whose ordering is
the whole answer:

```c
if (keepBelow())        return BelowLayer;
if (isActiveFullScreen()) return ActiveLayer;
if (keepAbove())        return AboveLayer;
return NormalLayer;
```

Two facts follow, and both were read out of the shipped 6.7.5 sources rather than inferred:

1. **`isActiveFullScreen()` is tested before `keepAbove()`.** A focused fullscreen window is on
   `ActiveLayer`, which is above `AboveLayer`. So `set_always_on_top(true)` cannot put the overlay
   over an active fullscreen game — and the restacking in ADR 0008 cannot either, because
   `Workspace::constrainedStackingOrder()` sorts purely by layer index: the layer *is* the z-axis,
   and there is no "above window X" concept that crosses layers.
2. **`isActiveFullScreen()` requires the fullscreen window to be the active one** (`ac == this`,
   or `ac` on another output, or `this` is a main window of `ac`). A fullscreen game that does not
   have focus is therefore *not* on `ActiveLayer` — it falls through to `keepAbove`/`NormalLayer`
   and the ordinary restacking applies to it. This is why a windowed game behaves differently
   from an active fullscreen one rather than less well: the rule really is conditional on focus.

The only bands above `ActiveLayer` are `NotificationLayer`, `ActiveLayer`'s superiors
(`PopupLayer`, `CriticalNotificationLayer`, `OnScreenDisplayLayer`) and `OverlayLayer`. Reaching
one of them means either a KWin window rule on the user's machine (`layer=overlay`, which was
tested and works) or `wlr-layer-shell`'s `overlay` layer (ADR 0003). Both have a real cost:
anything in `OverlayLayer` is above *every* fullscreen window, including other applications', so
an overlay parked there is no longer "above the game and below what the player raises" — it is
above everything, permanently.

There is also an operational half to this. The watcher in ADR 0008 re-asserts `set_always_on_top`
on a 400 ms loop. Against an active fullscreen game that request does not achieve anything, so the
app would be reporting a state it does not have: the overlay is behind the game while the app
believes it is showing.

## Decision

**While another client holds a fullscreen window, the watcher drops keep-above and re-shows the
window.** `OverlayPlacement` (in `src-tauri/src/x11_overlay.rs`) carries the fullscreen answer
back from the same X connection that does the restacking, so this costs no extra round trip;
`desired_keep_above(other_fullscreen, game_anchored)` is the whole rule, and it is a pure function
with tests. `apply_overlay_stacking` only calls `set_always_on_top` on a transition, because the
flag is a window property and 2.5 property writes a second for an unchanged value is waste.

The consequence is deliberate and is the reason this ADR exists rather than a one-line comment: on
a fullscreen game **the hotkey-raised overlay is a window behind the game that the player can
raise, read, and dismiss** — not a window that silently does nothing while still swallowing mouse
clicks (it is click-through in overlay mode, so an invisible overlay would also be invisible as a
problem, which is the worst failure mode available).

Fullscreen support is therefore explicitly **not delivered by this design**. It is delivered by
one of:

- `wlr-layer-shell`'s `overlay` layer (ADR 0003), which covers other applications' fullscreen
  windows as a consequence; or
- a documented, opt-in KWin window rule (`layer=overlay`), which was verified end to end on
  KWin 6.7.5: with the rule applied the app's window reports `layer=9` (`OverlayLayer`) while
  another client's fullscreen window reports `layer=5` (`ActiveLayer`), so it stays on screen
  over a fullscreen game. `tools/overlay_fullscreen_rule.sh` installs and removes it.

Both are user-visible changes to the whole desktop's stacking, which is why neither is the
**default**.

### The fullscreen claim is settled by pixels, not by layer numbers

Layer numbers are a compositor-internal report; they do not by themselves prove that an image
reaches the screen. Both halves were therefore measured against a deliberately active fullscreen
client (`/tmp/fs/activetest.sh`, `/tmp/fs/two.py`), screenshotting the whole screen and counting
non-backdrop pixels (`/tmp/fs/countpix.py`):

| Configuration | Non-backdrop pixels on a 3840x2160 screen | Overlay visible |
| --- | --- | --- |
| Active fullscreen client, no rule | `0` (0%, screen 100% client) | **no** |
| Active fullscreen client, rule installed | `114000` (1%) | **yes** |

The screenshot for the second row shows the build order drawn over the fullscreen client, so the
rule is what makes it work rather than an artefact of window ordering.

**The activation detail is the trap, and it invalidates a whole class of naive test.** KWin
promotes a client to `ActiveLayer` only while it is fullscreen *and active*. A test that starts the
app after the stand-in game leaves the app holding focus, so the "fullscreen" client silently
falls back to `NormalLayer` — where ordinary keep-above wins — and the overlay measures as visible
**with no rule installed at all** (measured: a crop showing the overlay drawn over the red client).
That result looks like proof of fullscreen support and is the opposite. Any future check must
activate the fullscreen client after the app is running, or it is measuring windowed behaviour.

### The rule is case-sensitive in a way that is easy to get wrong

KWin matches a rule's `wmclass` against `Window::resourceClass()` (or, with
`wmclasscomplete`, `resourceName` + `' '` + `resourceClass`), and `Rules::matchWMClass`
(`src/rules.cpp:365`) compares **case-sensitively** for both `ExactMatch` and `SubstringMatch`.
The app on the X11 backend reports `WM_CLASS(STRING) = "sc2-science-vessel", "Sc2-science-vessel"`.

So any rule written from how the app names itself — `wmclass=sc2-science-vessel`, which is
what a first attempt used — matches the *resource name* and silently never fires, because the
class is capitalised. Worse, it appears to work at first: without the rule the window is in
`AboveLayer` (3) because of keep-above, which looks like a success if you only check
"is it above the game". The rule that actually works is the capitalised
`wmclass=Sc2-science-vessel` (or a lowercase `sc2-science` substring, which matches the
capitalised class case-insensitively at that position because the string happens to be
lowercase after the first character).

This was found by trying the variants in one pass and reading KWin's layer back for each
(`/tmp/fs/rulefix.sh`): the first two variants sat at `layer=3`, the capitalised ones reached
`layer=9`.

## Consequences

- The app no longer lies about a state it does not hold, and the overlay is recoverable (raise it)
  rather than invisible-but-present on a fullscreen game.
- Borderless/windowed fullscreen — which is the mode most players who use overlays already
  choose — keeps working exactly as before, because a windowed game stays on `NormalLayer` and
  the ADR 0008 restacking applies.
- When the game drops out of fullscreen (including alt-tab to another application that then
  becomes active, since `isActiveFullScreen` is focus-conditional), keep-above returns on the next
  poll tick — at most 400 ms later.
- `fullscreen_state`'s three-variant enum was collapsed to a single `another_client_is_fullscreen()
 -> bool`, because the `SelfFullscreen` case was dead: the app never fullscreens its own window,
  and its own fullscreen would be a reason to keep being an overlay rather than to stop. A variant
  no caller can reach is a claim the tests cannot check.
- The app's own window is identified by title prefix `sc2-science-vessel` when filtering the walk.
  This is the same fragile title matching ADR 0008 already relies on for the game, and it is worth
  noting as a shared weakness: a renamed window would make the app consider itself a fullscreen
  client and stop being an overlay.

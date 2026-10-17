# 15. Raise the overlay above a fullscreen game with override-redirect, not a window rule

- Status: accepted
- Date: 2026-10-17
- Supersedes: `0011-yield-the-overlay-to-a-fullscreen-client.md` (the yielding half only)

## The problem

ADR 0011 established that the overlay cannot cover an **active** fullscreen game on KDE Wayland,
and made the app yield instead: it dropped its keep-above claim and re-showed the window, leaving
the player to find the app behind the game. The reasoning was sound — `Window::belongsToLayer()`
in KWin checks `isActiveFullScreen()` before `keepAbove()`, so a focused fullscreen client sits on
`ActiveLayer` (5) and a keep-above toplevel can only reach `AboveLayer` (3). Stacking order sorts
purely by layer index, so nothing a `keepAbove` window can do reaches above it.

The escape hatch was a KWin window rule forcing `layer=overlay` (9). That works, and
`tools/overlay_fullscreen_rule.sh` installs it, but it is a **desktop-wide setting the player has
to apply by hand**, and it puts the overlay above *every* fullscreen window on the machine, not
just the game. Shipping that as the only answer was not good enough.

## What was measured

Every number below comes from the reference machine (KDE Plasma 6.7.5, KWin 6.7.5, XWayland `:0`,
3840x2160), reading the compositor's own opinion back with
`qdbus6 org.kde.KWin /KWin org.kde.KWin.getWindowInfo`, and counting pixels on a full-screen
screenshot with a red fullscreen client as the backdrop.

| Mechanism | KWin layer | Visible over an **active** fullscreen client |
| --- | --- | --- |
| ordinary managed window, `keepAbove` | 3 (Above) | **0 %** |
| managed window, KWin rule `layer=overlay` | 9 (Overlay) | 13 % (the whole window) |
| **override-redirect (unmanaged) window** | unmanaged | **8.44 %** — exactly its own area |

The third row is the finding. `Window::belongsToLayer()` returns `OverlayLayer` for
`isUnmanaged()` **before** it ever consults `isActiveFullScreen()`, so an unmanaged window is above
a focused fullscreen client with **no configuration at all**. An override-redirect window is
exactly an unmanaged one.

A fourth measurement explains why the app kept failing to stay on top even when it did claim the
top: **activating any other window demotes a fullscreen client** from `ActiveLayer` (5) to
`NormalLayer` (2). Verified directly — with a keep-above window shown over a fullscreen client, the
client dropped to layer 2 and the keep-above window at layer 3 was above it, and it stayed that way
(three consecutive readings).

## The decision

While **another client holds an active fullscreen window**, the app makes its own window
override-redirect. It becomes a managed window again the moment either:

- the game stops being the active window (the player raised something over it), or
- the app switches to analysis mode, or
- the overlay is hidden.

This is implemented as `x11_overlay::set_unmanaged(window, bool)`, exposed through
`platform::set_unmanaged` so `lib.rs` names no GTK type, and driven from
`apply_overlay_stacking` on each 400 ms stacking tick.

### Why it is not done in place

`override_redirect` is fixed when the X window is created. Measured:

- flipping it on a **mapped** window does nothing — KWin decided on first sight and a mapped window
  never asks again; the window stayed at layer 2.
- flipping it while **unmapped in raw X** (`unmap` → `change_attributes` → `map`) left the window
  mapped but **unpainted** (0 % visible).
- **GTK's own `hide()` / `show()` works**, in both directions: managed 10.84 %, override-redirect
  8.44 %, managed again 10.84 %.

GDK tears the X resource down on hide and creates it again on show, so the flip lands on the
`CreateWindow` attributes. That is why `gtk_unmanaged` calls `gtk_window.hide()` and takes
`gtk_window.window()` while it exists, and why visibility is preserved across the flip — the flip
must not change whether the player can see the window.

### Why it is tied to the game being *active*, not merely fullscreen

`other_fullscreen` stays true while the game is fullscreen even after the player raises something
over it, and becoming unmanaged then would cover the window they just raised. That is the
requirement from the first message: "on top of the game window, but able to go behind other windows
if those are focused above the game window". An unmanaged window cannot go behind anything. So the
claim is taken only when the game is the active window, which is exactly when it is on the
compositor's `ActiveLayer` and needs out-ranking.

### The release must not leave a keep-above claim behind

The flip re-shows the window, because GDK has to recreate the X resource, and showing a window
raises it to the top of the stack. Releasing the override-redirect state for a window the player
has just raised something over therefore put the overlay *above* that window — measured directly in
the raw X stacking order: `game → RAISED → overlay`, exactly backwards. Two corrections: the caller
re-anchors after a release so the overlay goes back to sitting directly above the game, and
`claim_unmanaged` no longer sets `always_on_top` at all.

The second one was the subtler bug. `always_on_top` is a compositor **layer**, so `AboveLayer`
beats any ordinary window (`NormalLayer`) no matter what the X stack says — measured: the overlay
sat at `layer=3 keepAbove=true` above `RAISED` at `layer=2`. It also desynchronised the stacking
watcher, which tracks the flag's value to avoid re-sending it every tick: something else set the
flag, so the watcher's belief never changed and it never saw a reason to clear it. An
override-redirect window needs no claim anyway — KWin puts an unmanaged window above every managed
layer regardless — so the claim is now owned by exactly one place, the watcher, which starts from
`false` and only sets it for a windowed game.

### Focus must be refused, and before the window is mapped

An overlay that accepts keyboard focus **demotes the game it is drawn over**, which the app then
reads as "the game is not active any more", releases the unmanaged state, drops behind the game,
which makes the game active again. Measured as a visible oscillation (~25 polls in one state, ~14
in the other).

`set_accept_focus(false)` and `set_focus_on_map(false)` only influence whether a window takes focus
**when it is mapped**, so they are applied *before* `show()`. Measured: with the call after
`show()`, the overlay became `_NET_ACTIVE_WINDOW` and the game was demoted; with it before,
`_NET_ACTIVE_WINDOW` stayed on the game.

Analysis mode calls the inverse (`platform::allow_focus`), because it is an ordinary window the
player clicks and types into and a stale flag would make it neither raise nor accept a keystroke.

### GTK is a direct dependency now

`gtk = "0.18"` in `Cargo.toml`. `gtk_window()` hands back a `gtk::ApplicationWindow`, and the one
operation needed — `set_override_redirect` — is on **GDK** rather than GTK. Tauri already links
GTK 3 through tao, so this declares the same major version and shares the already-loaded library
instead of adding one.

## Consequences

**Good.** Both halves of the requirement are verified. Above an active fullscreen game: the overlay
is override-redirect and sits above the game in the raw X stacking order. After the player raises a
window over that game: the overlay becomes managed again, is restacked to `game → overlay →
raised`, and reports `keepAbove=false` at `layer=2`, i.e. genuinely behind the raised window. The
overlay also follows the game when it moves — anchoring measured at three different game positions
with a constant 24 logical-pixel inset from its top-left corner.

**Good.** The overlay now reaches above an active fullscreen game with **no user configuration on
any EWMH desktop**, not just KWin — X11 has no equivalent of the KWin layer model, so a compositor
that stacks override-redirect windows on top (Xorg, Xfce, KWin/X11, mutter/X11) needs no rule
either. Verified end to end against the real StarCraft II client: the overlay sat above a
fullscreen 3072x1728 game across 21 consecutive polls with `game_active=true` and the game
remaining on layer 5 throughout.

**Cost.** While the claim is held the window is unmanaged, so it has no decorations, no taskbar
entry and cannot be restacked by the window manager. That is acceptable for exactly as long as the
game is the active fullscreen window — the player is looking at the game, not at the desktop — and
it is released on every other path.

**Risk that remains.** An override-redirect window over an exclusive-fullscreen game is still only
verified on KWin. `wlr-layer-shell`'s `overlay` layer would be the Wayland-native answer and would
also cover Hyprland/Sway/Niri/COSMIC (ADR 0003), but `gtk-layer-shell` is not installed on the
reference machine and `sudo` is unavailable there, so it could not be built or tested.

## Related

- `0011-yield-the-overlay-to-a-fullscreen-client.md` — the behaviour this replaces when the game is
  active; its analysis of the KWin layer model is still correct and is the reason this ADR exists.
- `0008-overlay-stacking-on-kde-wayland.md` — the X11 backend, restacking, and the portal hotkey.
- `0003-overlay-is-a-native-layer-shell-window.md` — the proposed layer-shell design, unimplemented.

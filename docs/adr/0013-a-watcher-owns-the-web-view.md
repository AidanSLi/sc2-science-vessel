# 13. A watcher process owns the web view, because a resident web view costs 259 MiB

- **Status:** Accepted
- **Date:** 2026-10-17
- **Deciders:** maintainer
- **Related:** ADR 0012 (the app is resident and hints with a critical notification),
  ADR 0004 (the live clock comes from the client API), ADR 0010 (the mode follows the screen list)

## Context

The user asked for the app to "automatically start & stop when the game starts & stops" (m04856),
and then clarified what those words mean (m06360):

> "when I say 'the game starts' or 'the game exits', I am referring to the whole Starcraft 2
> process. When I say 'the match starts' or 'the match ends' I am referring to a match inside of
> SC2. … The start of the match is the moment the map loading screen disappears and the '3-2-1'
> counter starts. The end of the match is the moment the player presses the 'score screen' button."

and, critically for this decision:

> "This means that there should be no latency for the overlay when starting a match, as the web
> view has already been started on game (SC2) startup and it simply needs to change its window
> position and appearance."

A resident app that keeps a Tauri window alive is the obvious reading of that, and it was
implemented first. Its cost was then measured rather than assumed.

## The measurement, which decided this

Idle, window hidden, release build, on the reference machine (KDE Plasma 6 / Wayland, webkit2gtk
2.54.1). PSS, not RSS: three processes share WebKit's libraries, and RSS counts every shared page
once per process.

| process | PSS | RSS |
|---|---|---|
| `sc2-science-vessel` (main) | 123 MiB | 164 MiB |
| `WebKitWebProcess` | 101 MiB | 139 MiB |
| `WebKitNetworkProcess` | 34 MiB | 62 MiB |
| **total** | **259 MiB** | ~365 MiB |

The biggest single mapping in the main process is `libnvidia-gpucomp.so` at 17 MiB, followed by
WebKit, JavaScriptCore and LLVM. Nothing in the app's own code accounts for a meaningful part of it.

CPU, in contrast, is already negligible: **0.13 % of one core** while resident and hidden.

A dependency-free prototype of the same lifecycle, built as a separate binary, measured:

| | watcher prototype | resident Tauri app |
|---|---|---|
| PSS | **0.4 MiB** | 259 MiB |
| CPU | 0.00 % (below measurement) | 0.13 % |

**That is a factor of roughly 600.** A web view cannot be made small: the entire point of the app is
to render HTML, and WebKit's baseline cost is what it is. So "resident" and "extremely small" can
only both be true if the resident part is not the web view.

## Decision

The app is **one binary with two roles**, chosen by the command line:

- **watcher** (no arguments) — the resident process. Polls for the StarCraft II process once a
  second and owns a GUI child. ~0.4 MiB.
- **GUI** (`--gui`) — spawned by the watcher. The Tauri app. ~259 MiB, and it only exists while
  SC2 does.

The two are one executable rather than two because there is then nothing extra to install and the
two can never drift out of step with each other.

### The lifecycle

| event | detected by | effect |
|---|---|---|
| game starts | watcher, by process name | spawn the GUI, **hidden** |
| match starts | GUI, from the client API's screen list | show it, in overlay mode |
| match ends | GUI, from the client API's screen list | show it, in analysis mode |
| game exits | watcher, by process name | terminate the GUI |

Splitting the detection this way is what makes the latency requirement achievable. The GUI is
already running before the match begins, so at the 3-2-1 countdown it has nothing to do but change
window flags — no process start, no WebKit initialisation, no page load.

The two questions are answered by different mechanisms on purpose: "is SC2 running" cannot be asked
of the client API, because the client API does not exist until SC2 starts; and "is a match being
played" should not be guessed at from process names, because the client API reports it exactly
(ADR 0010).

### Ownership decides retirement

A GUI is either started **for a game** or **for the player**, and the watcher only retires the first
(`GuiOrigin` in `src-tauri/src/watcher.rs`). The first implementation retired any GUI whenever SC2
was absent, which meant launching the app with no game open produced a window that vanished on the
next poll. The distinction is a two-value enum rather than a comment because it is the kind of rule
that gets re-broken by a refactor.

### What the watcher does not do

It never changes the window's mode, never shows the guide, and never reads the client API. Those
belong to the GUI, which can see the screen list. Coupling the watcher to the client API would mean
two processes deciding the same fact.

## Consequences

**Good**

- ~0.4 MiB resident instead of 259 MiB, which is what makes residency defensible at all.
- A crashed or hung GUI does not take the watcher down; the next poll starts a fresh one.
- The GUI can be restarted (during development, or after a bad state) without the watcher caring.

**Costs, accepted**

- Two processes, and therefore a control channel between them (`src-tauri/src/control.rs`) plus a
  single-instance lock so the second launch does not start a second watcher.
- The GUI inherits its environment from the watcher, so the renderer and backend choices made in
  `main.rs` apply to the child automatically. This is a consequence, not an accident, and it is why
  the child is spawned directly rather than through the desktop's launcher machinery.
- The web view is cold on the *first* launch per game session. Measured window appearance: ~8 s cold,
  ~1 s warm. This does not affect the match-start latency the user asked about, because that launch
  happens when SC2 starts, which is minutes before a match.
- The portal hotkey is registered by the GUI, so the hotkey does not exist while SC2 is closed. This
  matches the spec: m06360 describes pressing the hotkey during a game or with the app open, not
  with nothing running.

**Still open**

- The watcher detects the game by process name on Linux only (`/proc`). Windows and macOS return
  `false` and the watcher stays idle — deliberately, so it cannot spawn web views it could never
  retire. Porting it needs `EnumProcesses`/`GetModuleBaseNameW` and `libproc` respectively.
- A stale `~/.config/autostart/*.desktop` from a development build points at `target/debug/…` and
  will fail at login once that path is rebuilt away. Enabling launch-at-login is refused for
  uninstalled builds for this reason; use `tools/install_dev.sh`.

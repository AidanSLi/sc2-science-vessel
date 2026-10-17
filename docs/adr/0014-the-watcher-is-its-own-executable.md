# 14. The watcher is its own executable, because one ELF shares one set of libraries

- **Status:** Accepted
- **Date:** 2026-10-18
- **Deciders:** maintainer
- **Related:** ADR 0013 (a watcher process owns the web view), ADR 0012 (the app is resident),
  ADR 0001 (the Python sidecar, which is the other `externalBin`)

## Context

ADR 0013 established that the web view cannot be resident, because it costs 259 MiB and the user
asked for a process whose footprint is "extremely small". The first implementation put both roles
in one executable and chose between them by the command line: no arguments meant the watcher,
`--gui` meant the window.

That met the letter of the requirement — the window really is absent while SC2 is closed — but a
measurement showed it did not meet the spirit of it.

### Measurement

`smaps_rollup` PSS for the watcher role, idle, no game running, on the reference machine
(Arch, KDE 6.7 Wayland, glibc):

| build | PSS | RSS | threads |
|---|---|---|---|
| one executable, both roles (the shared ELF) | **23.6 MiB** | 30.5 MiB | 4 |
| the split watcher crate, standard library only | **0.5 MiB** | 2.5 MiB | 2 |

Menus and CPU were never the problem — measured at 0.00–0.4 % of one core either way — so this
was purely a memory question, and 23.6 MiB is not "extremely small" when 0.5 MiB is available.

### Why one executable could not get below that

An ELF maps every shared library its code *can* reach, and the kernel does not know which
branches are taken. Because the watcher lived in the same binary as the window, its address space
included `libwebkit2gtk-4.1`, `libjavascriptcoregtk-4.1` and the GTK stack, and the process
resident-mapped a large share of them from the first `getpid`. The watcher never calls a single
one of those functions; it reads `/proc` once a second and blocks on a socket.

The link-time alternatives were all worse:

- **`-Wl,--gc-sections` / LTO cannot help.** They remove code the linker can prove unreachable
  *within* the crate graph; WebKit is a dynamic dependency that Tauri itself calls, so the link
  edge exists and must stay.
- **`dlopen` the GUI's dependencies lazily** would mean reimplementing the WebKitGTK
  initialisation sequence by hand, on a toolkit whose DMABUF renderer already has two open
  upstream crashes (ADR 0005).
- **`fork` after start-up** would inherit the mapped libraries in the child, and still leave the
  parent holding them.

The only way for a process to not map a library is for the library to not be in its dependency
graph. That means a separate executable.

## Decision

**The watcher is a separate, dependency-free crate: `src-tauri/watcher-bin/`, binary name
`sc2sv-watch`.**

1. It declares **no dependencies at all** — not Tauri, not WebKit, not GTK, not even `serde`.
   Its `Cargo.toml` says so explicitly, because that emptiness is the entire reason it exists.
2. Its logic is **not duplicated**. `main.rs` includes `control.rs`, `game_process.rs`,
   `gui_control.rs`, `single_instance.rs` and `watcher.rs` from `../src/` by `#[path]`, so there
   is exactly one copy of the supervisor's logic, tested by both crates (22 of the tests run in
   both). A copy would drift, and the disagreement would be invisible — the two roles would
   silently start differing on which process names count as the game.
3. It is built **always in release**, even when the app is built with `--debug`, because the size
   requirement is about this process and its profile is `lto`, `opt-level = "s"`, `panic = "abort"`
   and `strip`. Measured output: **376 248 bytes**.
4. It ships through **`externalBin`**, which is the only Tauri mechanism that installs a second
   executable, and it installs into the same directory as the GUI — which is how the watcher
   finds the window (a sibling lookup, no configuration).
5. `tools/build_watcher.py` builds it and stages it under the host target triple, because that is
   the filename `externalBin` resolves. `--check` lets CI fail before bundling rather than during.
6. **The GUI no longer chooses a role.** `src-tauri/src/main.rs` always runs the window.
   `--gui` is still accepted and ignored so a desktop entry written before the split still starts
   the right thing.

## Consequences

- **The resident process is ~48x smaller** (0.5 MiB against 23.6 MiB), which is what makes
  "resident" acceptable to the user rather than a compromise.
- **Two files must be installed together.** Both `tools/install_dev.sh` and the Tauri bundles do
  this, and the sibling lookup is the only ordering constraint between them.
- **The `.desktop` entry and the `PATH` symlink name `sc2sv-watch`, not the GUI.** Running "the
  app" must start the supervisor; starting the GUI directly would produce an unsupervised window
  that nothing ever retires.
- **"Launch at login" could no longer use `tauri-plugin-autostart`**, because that plugin records
  `std::env::current_exe()`, which is the GUI — the settings page is what asks. A login entry
  naming the GUI starts a window with no game detection. `src-tauri/src/autostart.rs` writes the
  entry itself, naming the watcher sibling, for Linux only, and returns an explicit error
  elsewhere rather than doing nothing silently.
- **The tray's "Quit" must reach the watcher**, or the watcher would see its child exit and start
  a new one the next time SC2 appeared. `control::QUIT` and `control::WATCHER_CONTROL_NAME` live
  in `control.rs` (a file both crates include) instead of `watcher.rs` for that reason.
- **A third crate to build**, which is one more step in CI and in `install_dev.sh`, and one more
  file for a packager to include.

## Alternatives rejected

- **Keep one binary and accept 23.6 MiB.** Rejected: the user's requirement was explicit, and the
  cost of meeting it is one extra 376 KiB file.
- **Two binaries with the watcher's source copied into its crate.** Rejected: two copies of
  process matching and the control protocol, guaranteed to drift, with no test able to notice.
- **A shell script or a systemd user unit as the supervisor.** Rejected: it cannot own the child's
  lifecycle portably, cannot expose the control socket, and adds an install-time dependency on a
  unit file's correctness on three platforms.

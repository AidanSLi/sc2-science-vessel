# Packaging and distribution

How a build becomes something a player can install, which formats ship for which system, and the
things that silently break an installer if they are forgotten.

This is a record of decisions and their reasons. The build itself is
`.github/workflows/release.yml`.

---

## 1. What a release actually is

The repository holds a **recipe**, not an installer. Tauri's bundler takes the compiled binary,
`tauri.conf.json`'s metadata and the frozen sidecar, and emits packages into
`src-tauri/target/release/bundle/`:

| Format | Path | Installs with |
| --- | --- | --- |
| Debian package | `bundle/deb/*.deb` | `dpkg -i` / `apt install ./…deb` |
| AppImage | `bundle/appimage/*.AppImage` | nothing — `chmod +x` and run |
| NSIS installer (Windows) | `bundle/nsis/*-setup.exe` | double-click |

`target/` is gitignored, so **no installer is ever committed**. What gets committed is the
workflow that produces them and uploads them to a GitHub Release.

---

## 2. One runner per operating system, not per distribution

Tauri links against the platform's own WebKitGTK / WinRT, so cross-compilation is not possible.
The next best thing is a CI runner per platform, which is what `release.yml` does: `ubuntu-22.04`
and `windows-latest`. macOS is deliberately absent for now.

**Linux needs only one runner.** The bundler emits `.deb` and `.AppImage` from a single build, so
there is no such thing as a separate "Arch runner". Tauri's `--bundles` accepts exactly three
values on Linux:

```
$ pnpm tauri build --bundles pacman
error: invalid value 'pacman' for '--bundles [<BUNDLES>...]'
  [possible values: deb, rpm, appimage]
```

This project ships `deb` and `appimage`. `rpm` is supported by the bundler but omitted: an RPM
built on Ubuntu is worse than useless to Fedora users, who are better served by the AppImage until
somebody builds an RPM on Fedora proper.

### Which format for which distribution

| Distribution family | Format | Why |
| --- | --- | --- |
| Debian, Ubuntu, Mint, Pop!_OS | `.deb` | native, dependency-managed |
| **Arch, EndeavourOS, Manjaro** | **`.AppImage`** | the `.deb` will not install |
| Fedora, RHEL, openSUSE | `.AppImage` (or a future `.rpm`) | as above |
| Windows | `.exe` (NSIS) | single self-contained installer |

**Arch cannot use the `.deb`.** Not because the binary inside it would not run, but because the
dependency names are Debian's:

```
Depends: libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0
```

Arch calls those `libayatana-appindicator`, `webkit2gtk-4.1` and `gtk3`. The AppImage sidesteps
this entirely by bundling its own libraries.

### The AppImage caveat on current Arch

AppImage is the most volatile of the three formats. Tauri issue
[#15665](https://github.com/tauri-apps/tauri/issues/15665) reports that AppImages built with default
settings can start *headlessly* on Mesa 25+ / glib 2.88+ systems — the process runs, no window ever
appears, and nothing is logged. The cause is a combination of over-bundled
`libwayland`/glib/gstreamer and a stale `GST_PLUGIN_SYSTEM_PATH` that the AppImage's `AppRun`
exports unconditionally.

This was **tested on the reference machine** (Arch, Mesa 26.2.4, glib2 2.90.1 — precisely the
broken configuration) and the AppImage built from this repository **works**: the WebKit helper
processes start and the window renders. So AppImage is viable here, but it is the format most
likely to need attention after a system upgrade, and the first thing to suspect if a user reports
"it launches but I see nothing" on Arch.

An AUR package would be the properly integrated Arch option; that means writing a `PKGBUILD` by
hand, since the CLI has no pacman target.

### Building on the oldest base you intend to support

`ubuntu-22.04`, not a newer LTS. glibc compatibility is inherited from the build machine, so
building on a newer base silently raises the minimum glibc every user must have.

---

## 3. The Python sidecar must be bundled, or the app cannot analyse anything

This is the part that is easy to get wrong, because **getting it wrong produces a working
installer**.

The shell never parses a replay — `sc2reader` does, and it is a Python library (ADR 0001). An
installed app therefore has to carry a Python interpreter, and the only way to do that without
asking the user to install Python is to freeze one in. `tools/build_sidecar.py` does that with
PyInstaller.

Three details, each of which fails *at runtime* rather than at build time if missed:

1. **The filename must carry the target triple.** Tauri's `externalBin` resolves
   `sc2sv-sidecar-<triple>`, strips the triple when placing the file in the bundle, and puts it in
   `usr/bin/` (Linux) or next to the `.exe` (Windows). The script asks `rustc -vV` for the host
   triple rather than guessing, because a mismatch is a silent "sidecar not found" at run time.
2. **`--collect-data sc2reader` is mandatory.** `sc2reader` reads unit costs and build times from
   three JSON files (`data/{attributes,train_commands,unit_info}.json`). PyInstaller cannot see a
   data file it is not told about, so a missing one fails deep inside the library the first time a
   metric needs a cost.
3. **The entry point must be a module, not a file inside the package.** PyInstaller runs its entry
   script as `__main__`, and `sc2sv/cli.py` uses relative imports, which fails with
   `ImportError: attempted relative import with no known parent package`. `sidecar/sc2sv_sidecar_entry.py`
   is a shim that imports the CLI as a package module and calls the same `sc2sv.cli:main` the
   console script does.

### What was verified

- The frozen binary's JSON output is **byte-identical** to the repo virtualenv's for the same
  replay (16531 bytes each), so freezing does not change a single number.
- The sidecar extracted from a built `.deb` runs **standalone** — outside the repo, with no
  virtualenv and no interpreter on `PATH`.
- Removing the sidecar before a build makes the build **fail loudly**
  (`resource path 'sc2sv-sidecar-x86_64-unknown-linux-gnu' doesn't exist`) rather than producing a
  bundle without it. So CI cannot silently ship a broken installer, and neither can a developer.

### Reproducing locally

```bash
uv run --with pyinstaller python tools/build_sidecar.py   # writes src-tauri/sc2sv-sidecar-<triple>
pnpm tauri build --bundles deb,appimage
```

---

## 4. Platform support matrix for the overlay and the hotkey

The overlay and the global hotkey are the only two features that cannot be written once and used
everywhere, because neither "stay above that other application's window" nor "receive this key
while another application has focus" has a cross-platform protocol.

| Platform / desktop | Overlay above a game window | Above an **active fullscreen** game | Global hotkey while the game has focus |
| --- | --- | --- | --- |
| **Windows** | yes (restacking) | see §5 — unverified | yes (`tauri-plugin-global-shortcut`) |
| **Linux + X11**, any DE | yes (restacking) | only with the KWin rule, see §5 | yes (`XGrabKey`) |
| **Linux + KDE Wayland** | yes (via XWayland) | only with the KWin rule, see §5 | yes (XDG `GlobalShortcuts` portal) |
| **Linux + GNOME Wayland** | no — see §5 | no | no — see §5 |
| **Linux + other Wayland** (Hyprland, Sway, Niri, COSMIC) | no | no | depends on the portal |

The app reports which case it is in, rather than failing silently: `overlay_supported_command` is
exposed to the settings page.

### What the app does on an unsupported display server

It still runs, still analyses replays, still shows the build-order guide and still advances it on
the game clock. What it cannot do is *place* the guide next to the game window or guarantee it
stays above it. The stacking watcher reports this once, logs a line naming ADR 0008, and stops
(rather than re-asking several times a second for a capability that will never appear).

### Why Linux uses X11 rather than Wayland

On a native Wayland session GTK's `gdk_wayland_window_set_keep_above` is an **empty function**
(`gdk/gdkwayland/gdkwindow-wayland.c`), so `set_always_on_top` does nothing; `set_position` has no
protocol equivalent; and a Wayland client cannot read another application's window geometry at all.
All three are needed. On the X11 backend through XWayland all three work, which is why the app sets
`GDK_BACKEND=x11` when an X server is present (`main.rs`, ADR 0008).

---

## 5. Open questions and known limitations

These are recorded rather than papered over.

- **Windows is written but not verified on hardware.** No Windows machine was available, and no
  Windows target can even be linked here (no MSVC toolchain; `ring` fails its build script). The
  API semantics come from Microsoft's documentation. Treat the first Windows run as the
  verification step. See `src-tauri/src/platform/windows.rs`.
- **An *active* fullscreen game cannot be covered by an ordinary window on KDE Wayland.** KWin's
  `Window::belongsToLayer` checks `isActiveFullScreen()` *before* `keepAbove()`, so a focused
  fullscreen client sits on `ActiveLayer`, above every layer an always-on-top toplevel can reach.
  The escapes are `wlr-layer-shell`'s `overlay` layer, or a KWin rule setting `layer=overlay`.
  `tools/overlay_fullscreen_rule.sh install` applies that rule per-user; it is a documented
  escape hatch, not the shipped mechanism, and it means the overlay will also sit above other
  applications' fullscreen windows.
- **GNOME Wayland** implements neither the relevant part of the XDG `GlobalShortcuts` portal nor a
  layer a normal window can reach above fullscreen. See the per-desktop table above.
- **`--bundles rpm` is untested**: `rpmbuild` is not installed on the reference machine, so the
  bundler cannot produce one there.
- **macOS is out of scope for now.** Replay analysis would work, but SC2 on Apple Silicon is
  Rosetta-only and covering another application's fullscreen Space needs an `NSPanel` subclass
  that this project has not written.
- **The app's portal identity is the launching terminal's until it is installed.** The
  `GlobalShortcuts` portal resolves a host app id from a systemd scope named
  `app-<ApplicationID>-<random>.scope` plus an installed `<app_id>.desktop` file. A bundled
  install provides both, which is why the hotkey binding behaves correctly for an installed user
  and resolves to e.g. `org.kde.konsole` when run from a terminal during development.

---

## 6. Cutting a release

The workflow is manual on purpose, so a release is always a deliberate act:

1. Actions → **release** → *Run workflow*.
2. Enter a tag, e.g. `v0.1.0-beta.1`. The version comes from the tag; no commit is needed.
3. Leave *draft* checked to review the artefacts before publishing.

The `verify` job runs the frontend type-check, the frontend build and the Python test suite first,
so a broken commit costs seconds rather than two twenty-minute platform builds. The `linux` and
`windows` jobs then build in parallel, and `release` attaches every artefact to the release.

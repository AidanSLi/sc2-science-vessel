#!/usr/bin/env bash
# Installs the current build into ~/.local so it behaves like a real, installed application.
#
# ## Why not just run target/debug/sc2-science-vessel
#
# Three things only work when the app is installed:
#
# 1. **The XDG GlobalShortcuts portal resolves the app's identity** from a `.desktop` file plus
#    the systemd scope name. Launched from a terminal, the portal sees the *terminal's* app id,
#    so the hotkey it registers is filed under `org.kde.konsole` and the approval dialog says
#    "Konsole" instead of the app's name. Installing a `.desktop` file is what fixes that.
# 2. **Launch at login** is refused for an uninstalled build, by design (the autostart entry
#    records the executable's path, and a path into `target/` breaks on the next rebuild).
# 3. **The icons and window metadata** are read from the desktop entry.
#
# ## What it installs
#
#   ~/.local/lib/sc2-science-vessel/sc2sv-watch          the watcher (about 370 KiB, always running)
#   ~/.local/lib/sc2-science-vessel/sc2-science-vessel   the window (WebKit, started on demand)
#   ~/.local/lib/sc2-science-vessel/sc2sv-sidecar        the Python replay parser
#   ~/.local/bin/sc2-science-vessel                      a symlink to the watcher, so the menu,
#                                                        autostart and PATH all mean "the app"
#   ~/.local/share/applications/sc2-science-vessel.desktop
#   ~/.local/share/icons/hicolor/<size>/apps/sc2-science-vessel.png
#
# Everything is under $HOME, so no password is needed and nothing in /usr is touched. Re-running
# overwrites, which is what makes "always have the latest build" a matter of running this again.
#
# **Do not run this with sudo.** Two reasons, and the second is the one that matters: classically,
# root's $HOME is /root, so `sudo tools/install_dev.sh` would install a second copy of the app that
# no login session can see. And modern rustup installs the toolchain into the *user's* ~/.cargo,
# which is not on root's PATH, so the build dies at the first `rustc` with "command not found".
# The script refuses rather than producing either outcome.
#
# ## Usage
#
#   tools/install_dev.sh              build (debug) and install
#   tools/install_dev.sh --release    build optimised and install
#   tools/install_dev.sh --no-build   install whatever is already built
#   tools/install_dev.sh --uninstall  remove everything this script installed
#
# The debug build is the default because the point is to see UI changes quickly; a release build
# takes about two minutes and is what you want when checking performance. The *watcher* is always
# built optimised: it is the process that runs all session, and its size requirement is what the
# split into two binaries exists to meet.

set -euo pipefail

APP_NAME="sc2-science-vessel"
PRODUCT_NAME="SC2 Science Vessel"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

LIB_DIR="${HOME}/.local/lib/${APP_NAME}"
BIN_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor"

PROFILE="debug"
DO_BUILD=1
DO_UNINSTALL=0

for argument in "$@"; do
	case "$argument" in
		--release) PROFILE="release" ;;
		--no-build) DO_BUILD=0 ;;
		--uninstall) DO_UNINSTALL=1 ;;
		-h|--help)
			sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
			exit 0
			;;
		*)
			echo "unknown argument: $argument" >&2
			exit 2
			;;
	esac
done

# Installing per-user is the whole design, so running as root is refused instead of silently
# writing to /root/.local where the desktop will never look for it.
#
# The effective uid is the right test rather than $SUDO_USER: `sudo -E` can keep the caller's HOME
# while the process is still root, so a caller that dodged the uid check would write root-owned
# files into a real home directory. Checking the euid catches both spellings of the mistake.
if [ "${EUID}" -eq 0 ]; then
	cat >&2 <<'REFUSE'
This script installs into ~/.local for the current user, so it must not run as root.

As root, $HOME is /root: the app would be installed where no login session looks for it, and
the copy your menu and PATH refer to would be untouched. Building also needs the rust toolchain,
which rustup puts in the *user's* ~/.cargo/bin rather than root's PATH — that is the
"rustc: command not found" you get from sudo.

Run it without sudo:

  tools/install_dev.sh

Nothing here needs a password, because nothing outside your home directory is written to.
REFUSE
	exit 2
fi

# pnpm installs outside the default PATH on some systems, and an install script that fails with
# "pnpm: command not found" because of a shell profile is a bad first experience.
if ! command -v pnpm >/dev/null 2>&1; then
	export PATH="${HOME}/.local/share/pnpm/bin:${PATH}"
fi

# Same reasoning for the Rust toolchain: rustup's installer appends ~/.cargo/bin to a shell
# profile, which a non-interactive shell (or a desktop launcher, or CI before its setup step)
# has not sourced. A missing `rustc` later fails with a message that names neither the toolchain
# nor the fix, so it is resolved here where the remedy is known.
if ! command -v rustc >/dev/null 2>&1; then
	export PATH="${HOME}/.cargo/bin:${PATH}"
fi
if ! command -v rustc >/dev/null 2>&1; then
	echo "rustc is not on PATH and not in ~/.cargo/bin." >&2
	echo "Install the Rust toolchain from https://rustup.rs, then run this again." >&2
	exit 1
fi

uninstall() {
	echo "Removing ${PRODUCT_NAME} from ${HOME}/.local"
	rm -f "${BIN_DIR}/${APP_NAME}"
	rm -f "${DESKTOP_DIR}/${APP_NAME}.desktop"
	# The login entry names an absolute path inside LIB_DIR, so leaving it behind would make the
	# next login try to start a program that no longer exists.
	rm -f "${XDG_CONFIG_HOME:-${HOME}/.config}/autostart/${APP_NAME}.desktop"
	rm -rf "${LIB_DIR}"
	for size in 32x32 128x128 256x256@2 512x512; do
		rm -f "${ICON_DIR}/${size}/apps/${APP_NAME}.png"
	done
	command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "${DESKTOP_DIR}" || true
	echo "Done. Your settings in ~/.config/${APP_NAME} were left alone."
}

if [ "$DO_UNINSTALL" = "1" ]; then
	uninstall
	exit 0
fi

if [ "$DO_BUILD" = "1" ]; then
	cd "${REPO_ROOT}"

	# The sidecar is a separate artefact: PyInstaller freezes Python + sc2reader into one
	# executable, and Tauri's bundler expects it next to the binary under its target-triple name.
	# It changes far less often than the Rust code, so it is only rebuilt when missing or when
	# the caller asks by deleting it.
	TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
	SIDECAR="${REPO_ROOT}/src-tauri/sc2sv-sidecar-${TRIPLE}"
	if [ ! -f "${SIDECAR}" ]; then
		echo "Freezing the replay parser (first run only, takes a minute)…"
		uv run --with pyinstaller python tools/build_sidecar.py
	fi

	# Staged before `tauri build`, because the bundler copies `externalBin` entries and a missing
	# file fails the build rather than producing a bundle without it.
	python3 tools/build_watcher.py
	WATCHER="${REPO_ROOT}/src-tauri/sc2sv-watch-${TRIPLE}"
	if [ ! -x "${WATCHER}" ]; then
		echo "the watcher was not staged at ${WATCHER}" >&2
		exit 1
	fi

	if [ "$PROFILE" = "release" ]; then
		echo "Building ${PRODUCT_NAME} (release)…"
		pnpm tauri build --no-bundle
		BUILT="${REPO_ROOT}/src-tauri/target/release/${APP_NAME}"
	else
		echo "Building ${PRODUCT_NAME} (debug)…"
		pnpm tauri build --debug --no-bundle
		BUILT="${REPO_ROOT}/src-tauri/target/debug/${APP_NAME}"
	fi

	if [ ! -x "${BUILT}" ]; then
		echo "build did not produce ${BUILT}" >&2
		exit 1
	fi
else
	if [ "$PROFILE" = "release" ]; then
		BUILT="${REPO_ROOT}/src-tauri/target/release/${APP_NAME}"
	else
		BUILT="${REPO_ROOT}/src-tauri/target/debug/${APP_NAME}"
	fi
	if [ ! -x "${BUILT}" ]; then
		echo "no build at ${BUILT}; run without --no-build" >&2
		exit 1
	fi
fi

echo "Installing into ${HOME}/.local"
install -d "${LIB_DIR}" "${BIN_DIR}" "${DESKTOP_DIR}"

# A copy, not a symlink into target/: the next `cargo build` replaces the file that a symlink
# points at, which would leave a running app pointed at a half-written binary, and `target/` can
# be deleted at any time.
install -m 755 "${BUILT}" "${LIB_DIR}/${APP_NAME}"

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"

# The watcher is resolved by absolute path, not by a sibling lookup, because the installer puts
# both files in the same directory but a packaged install must not be assumed to.
WATCHER="${REPO_ROOT}/src-tauri/sc2sv-watch-${TRIPLE}"
if [ -x "${WATCHER}" ]; then
	install -m 755 "${WATCHER}" "${LIB_DIR}/sc2sv-watch"
else
	echo "warning: no watcher at ${WATCHER}; run tools/build_watcher.py" >&2
fi
SIDECAR="${REPO_ROOT}/src-tauri/sc2sv-sidecar-${TRIPLE}"
if [ -f "${SIDECAR}" ]; then
	install -m 755 "${SIDECAR}" "${LIB_DIR}/sc2sv-sidecar"
else
	echo "warning: no sidecar at ${SIDECAR}; replay analysis will not work" >&2
fi

# The symlink points at the **watcher**, not the window: running the app means "be resident and
# show a window", which is what the watcher does. Pointing at the GUI would skip game detection
# entirely and leave a second, unsupervised copy of the window able to start.
ln -sf "${LIB_DIR}/sc2sv-watch" "${BIN_DIR}/${APP_NAME}"

# `StartupWMClass` must match what the window actually reports, or the desktop will show a
# second, generic icon for the running app instead of associating it with this entry. The X11
# backend reports `Sc2-science-vessel` (capital S) — see ADR 0008 and tools/overlay_fullscreen_rule.sh,
# which depends on the same detail.
cat > "${DESKTOP_DIR}/${APP_NAME}.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=${PRODUCT_NAME}
Comment=Offline SC2 replay analysis and build-order coach
Exec=${LIB_DIR}/sc2sv-watch
Icon=${APP_NAME}
Terminal=false
Categories=Game;
StartupWMClass=${APP_NAME}
DESKTOP

# Tauri's icon filenames are not the same as the freedesktop theme directory names: it ships
# `128x128@2x.png` where a theme wants `256x256`, so each source is mapped to its target instead
# of assuming the names line up.
for pair in "32x32:32x32" "128x128:128x128" "128x128@2x:256x256"; do
	source_name="${pair%%:*}"
	theme_size="${pair##*:}"
	source_icon="${REPO_ROOT}/src-tauri/icons/${source_name}.png"
	if [ -f "${source_icon}" ]; then
		install -d "${ICON_DIR}/${theme_size}/apps"
		install -m 644 "${source_icon}" "${ICON_DIR}/${theme_size}/apps/${APP_NAME}.png"
	fi
done

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "${DESKTOP_DIR}" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "${ICON_DIR}" >/dev/null 2>&1 || true

cat <<DONE

Installed. The app is now in your application menu as "${PRODUCT_NAME}",
and on PATH as ${APP_NAME}.

  Start it         ${APP_NAME}
  The window       press the overlay hotkey (default Ctrl+Alt+O)
  Reinstall        tools/install_dev.sh
  Reinstall fast   tools/install_dev.sh --no-build
  Remove           tools/install_dev.sh --uninstall

The running instance is a watcher: it waits for StarCraft II, starts the window when the game
does, and quits the window when the game exits. It is a separate 370 KiB binary so that waiting
costs almost nothing; the window and its web view only exist while you are using them.
DONE

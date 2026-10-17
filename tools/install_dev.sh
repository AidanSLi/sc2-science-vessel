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
#   ~/.local/lib/sc2-science-vessel/sc2-science-vessel   the app (watcher + GUI, one binary)
#   ~/.local/lib/sc2-science-vessel/sc2sv-sidecar        the Python replay parser
#   ~/.local/bin/sc2-science-vessel                      a symlink, so it is on PATH
#   ~/.local/share/applications/sc2-science-vessel.desktop
#   ~/.local/share/icons/hicolor/<size>/apps/sc2-science-vessel.png
#
# Everything is under $HOME, so no password is needed and nothing in /usr is touched. Re-running
# overwrites, which is what makes "always have the latest build" a matter of running this again.
#
# ## Usage
#
#   tools/install_dev.sh              build (debug) and install
#   tools/install_dev.sh --release    build optimised and install
#   tools/install_dev.sh --no-build   install whatever is already built
#   tools/install_dev.sh --uninstall  remove everything this script installed
#
# The debug build is the default because the point is to see UI changes quickly; a release build
# takes about two minutes and is what you want when checking performance.

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

# pnpm installs outside the default PATH on some systems, and an install script that fails with
# "pnpm: command not found" because of a shell profile is a bad first experience.
if ! command -v pnpm >/dev/null 2>&1; then
	export PATH="${HOME}/.local/share/pnpm/bin:${PATH}"
fi

uninstall() {
	echo "Removing ${PRODUCT_NAME} from ${HOME}/.local"
	rm -f "${BIN_DIR}/${APP_NAME}"
	rm -f "${DESKTOP_DIR}/${APP_NAME}.desktop"
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
SIDECAR="${REPO_ROOT}/src-tauri/sc2sv-sidecar-${TRIPLE}"
if [ -f "${SIDECAR}" ]; then
	install -m 755 "${SIDECAR}" "${LIB_DIR}/sc2sv-sidecar"
else
	echo "warning: no sidecar at ${SIDECAR}; replay analysis will not work" >&2
fi

# The symlink is what puts the app on PATH. The sidecar is looked up next to the real executable
# (see src-tauri/src/sidecar.rs), so it does not need its own entry.
ln -sf "${LIB_DIR}/${APP_NAME}" "${BIN_DIR}/${APP_NAME}"

# `StartupWMClass` must match what the window actually reports, or the desktop will show a
# second, generic icon for the running app instead of associating it with this entry. The X11
# backend reports `Sc2-science-vessel` (capital S) — see ADR 0008 and tools/overlay_fullscreen_rule.sh,
# which depends on the same detail.
cat > "${DESKTOP_DIR}/${APP_NAME}.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=${PRODUCT_NAME}
Comment=Offline SC2 replay analysis and build-order coach
Exec=${LIB_DIR}/${APP_NAME}
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
does, and quits the window when the game exits. Its own memory use is about 400 KiB.
DONE

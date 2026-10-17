// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// What this process was asked to be.
///
/// The app is one binary with two roles, chosen by the command line rather than shipped as two
/// programs. The resident role costs about 400 KiB of memory; the GUI role costs about 260 MiB
/// because it carries a web view. Shipping them as one file means there is nothing extra to
/// install and the two can never drift out of step, which is the whole reason for the split
/// (ADR 0013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
	/// Supervise the GUI: start it when StarCraft II starts, retire it when the game exits.
	Watch,
	/// Be the GUI. Spawned by the watcher, or by hand for development and for tests.
	Gui,
}

/// Reads the requested role from the command line.
///
/// `--gui` is the only way to get a window directly. Everything else — no arguments, a desktop
/// entry, an autostart entry, a second click on the app icon — is the watcher, because that is
/// what those launches mean. An unrecognised argument is ignored rather than fatal, so a desktop
/// entry that passes a stray `%U` still starts the app.
///
/// @returns The role this process should play.
fn requested_role() -> Role {
	if std::env::args().any(|argument| argument == "--gui") {
		return Role::Gui;
	}
	Role::Watch
}

/// Whether this launch is the desktop session's autostart entry.
///
/// The autostart entry passes `--watched-by-desktop`, which is the difference between "start
/// silently and wait for the game" and "the player just clicked this, show them a window".
///
/// Detected from the command line rather than from a setting because it describes *this launch*,
/// not a preference: the same installed executable is both an autostart entry and a menu item.
///
/// @returns `true` when the launcher asked for a silent start.
fn started_by_autostart() -> bool {
	std::env::args().any(|argument| argument == "--watched-by-desktop")
}

fn main() {
	suppress_broken_webview_renderer();
	prefer_x11_when_xwayland_is_present();

	match requested_role() {
		Role::Gui => sc2_science_vessel_lib::run(),
		Role::Watch => {
			let code = supervise();
			// `std::process::exit` rather than returning, because GTK may have installed exit
			// handlers that would otherwise run in a watcher that never initialised GTK.
			std::process::exit(code)
		}
	}
}

/// Runs the resident supervisor, spawning this same executable with `--gui` for the window.
///
/// The GUI is started as a child of this process rather than through the desktop's launcher
/// machinery on purpose: a child inherits the environment already set up above (the renderer and
/// backend choices), and terminating it is then a plain `kill` with no dependency on D-Bus,
/// `systemd-run` or a window manager's cooperation.
///
/// A launch that finds another watcher already running does not start a second one; it asks the
/// running one to show the app, which is what clicking the app icon means.
///
/// @returns The watcher's exit code.
fn supervise() -> i32 {
	// Single instance. The lock is held for the life of the process and released by the kernel
	// on exit, so a crashed watcher does not leave a stale lock behind the way a pid file would.
	let Some(_lock) = sc2_science_vessel_lib::single_instance::acquire() else {
		if sc2_science_vessel_lib::watcher::request_show_of_running_instance() {
			eprintln!("sc2-science-vessel: already running; asked it to show the window");
			return 0;
		}
		// Another process holds the lock but did not answer on the control socket. It is
		// either wedged or from an older build; refusing to run is better than starting a
		// second supervisor that would fight it over the same window.
		eprintln!(
			"sc2-science-vessel: another instance is already running but did not respond; \
			 quit it before starting another"
		);
		return 1;
	};

	let intent = if started_by_autostart() {
		sc2_science_vessel_lib::watcher::StartIntent::Background
	} else {
		sc2_science_vessel_lib::watcher::StartIntent::Interactive
	};
	sc2_science_vessel_lib::watcher::run(intent, spawn_gui)
}

/// Starts the GUI as a child process.
///
/// The visibility is passed through as an environment variable rather than being left to the
/// user's `start_hidden` setting, because the watcher knows *why* the GUI is starting and the
/// setting does not: a GUI spawned because SC2 started must be silent, and one spawned because
/// the player clicked the app must be visible. `SC2SV_START_HIDDEN` overrides the setting for
/// this launch only, so choosing "show on start" in settings does not make every game launch
/// flash a window on screen.
///
/// @param start Whether the window should appear immediately.
/// @returns The child handle, or `None` when this executable's own path cannot be determined.
fn spawn_gui(start: sc2_science_vessel_lib::watcher::GuiStart) -> Option<std::process::Child> {
	let executable = std::env::current_exe().ok()?;
	let hidden = matches!(start, sc2_science_vessel_lib::watcher::GuiStart::Hidden);
	let child = std::process::Command::new(executable)
		.arg("--gui")
		.env("SC2SV_START_HIDDEN", if hidden { "1" } else { "0" })
		.spawn()
		.ok()?;
	eprintln!("sc2-science-vessel: started the app window (pid {})", child.id());
	Some(child)
}

/// Puts GTK on the X11 backend whenever XWayland is available.
///
/// The overlay has to be always-on-top and placed next to the game window, and **a native
/// Wayland client can do neither**: GTK3's `gdk_wayland_window_set_keep_above` is an empty
/// function, there is no protocol for a client to position its own toplevel, and one Wayland
/// client cannot read another's geometry. On the X11 backend all three work — keep-above,
/// `move`, and reading the game's rectangle — which is what makes the overlay possible.
///
/// StarCraft II under Wine is itself an XWayland window, so the X11 path is also the only
/// one where the game and the overlay can see each other at all. This is not a fallback for
/// missing functionality; it is the mechanism.
///
/// It is skipped when the user has already chosen a backend (an explicit `GDK_BACKEND`
/// always wins, so a Wayland-only setup or a debugging session is not overridden) or when
/// there is no X server to connect to, where forcing the X11 backend would leave the app
/// unable to start.
///
/// `unsafe` because environment mutation is not thread-safe; like the renderer setting this
/// runs on the main thread before any other thread exists.
fn prefer_x11_when_xwayland_is_present() {
	#[cfg(target_os = "linux")]
	{
		if std::env::var_os("GDK_BACKEND").is_some() {
			return;
		}
		if !sc2_science_vessel_lib::x11_overlay::xwayland_available() {
			eprintln!(
				"sc2-science-vessel: no X server, staying on the Wayland backend; the overlay \
				 cannot be kept above the game or placed next to it on Wayland"
			);
			return;
		}
		// SAFETY: called from `main` before GTK initialises and before threads are spawned.
		unsafe { std::env::set_var("GDK_BACKEND", "x11") };
		eprintln!(
			"sc2-science-vessel: using the X11 backend through XWayland, so the overlay can \
			 stay above the game window; see docs/adr/0008-overlay-stacking-on-kde-wayland.md"
		);
	}
}

/// Disables WebKitGTK's DMABUF renderer on Linux unless the user has already chosen.
///
/// WebKitGTK 2.54.1's DMABUF renderer fails on some Mesa/driver combinations and takes the
/// process down with it, under a message that names nothing actionable:
///
/// ```text
/// Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
/// ```
///
/// The related X11 symptom is a window that opens and never paints while
/// `Failed to create GBM buffer of size 1280x820: Invalid argument` repeats on stderr — those
/// messages alone do not prove the renderer is at fault, so they are recorded here as an
/// observation, not as a diagnosis.
///
/// This was reproduced on the reference machine (NVIDIA RTX 4070 plus an AMD Raphael iGPU,
/// KDE 6.7 Wayland). Doing it here rather than in `src-tauri/.cargo/config.toml` matters for
/// packaging, because a cargo `[env]` block does not reach a bundled deb, rpm, AppImage or
/// `.desktop` launch — those would crash on start-up for every installed user.
///
/// The cost is a little rendering performance, which is why an explicit setting from the
/// user always wins: `WEBKIT_DISABLE_DMABUF_RENDERER=0` restores the DMABUF path on a
/// machine whose drivers handle it.
///
/// `unsafe` because environment mutation is not thread-safe; this runs on the main thread
/// before any other thread exists, which is the one situation where it is sound.
fn suppress_broken_webview_renderer() {
	#[cfg(target_os = "linux")]
	{
		if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
			// SAFETY: called first thing in `main`, before threads are spawned.
			unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
			eprintln!(
				"sc2-science-vessel: using the CPU compositor path for WebKitGTK \
				 (WEBKIT_DISABLE_DMABUF_RENDERER=1). Set it to 0 to use DMABUF instead; \
				 see docs/adr/0005-disable-webkitgtk-dmabuf-renderer.md"
			);
		}
	}
}

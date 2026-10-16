// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    suppress_broken_webview_renderer();
    prefer_x11_when_xwayland_is_present();
    sc2_science_vessel_lib::run()
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

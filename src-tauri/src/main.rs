// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
	suppress_broken_webview_renderer();
	sc2_science_vessel_lib::run()
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

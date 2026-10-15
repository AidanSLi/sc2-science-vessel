<script lang="ts">
	/**
	 * Root layout: the only place global CSS is imported, so every route (including the
	 * overlay) inherits the theme tokens without importing them twice.
	 */
	import { onMount } from 'svelte';
	import { isTauri } from '#lib/client-api';
	import '../app.css';

	let { children } = $props();

	/**
	 * Tells the Rust shell that the frontend has painted, so it can reveal the main window.
	 *
	 * The window is declared `visible: false` in `tauri.conf.json` precisely so it cannot be
	 * seen empty, which means something has to ask for it. That is this, and it is why the
	 * call is here rather than in a page: every route renders inside this layout, so no page
	 * can forget it.
	 *
	 * It runs in `onMount` because that fires after the initial render, which is the point at
	 * which showing the window stops being a flash of blank surface. Failures are swallowed
	 * deliberately — if the command does not land, the watchdog in `lib.rs` shows the window
	 * after ten seconds regardless, and an unhandled rejection in a layout would be a worse
	 * outcome than a slightly late window.
	 *
	 * `isTauri()` guards the import so the same code runs in a plain browser tab during
	 * development, where `invoke` has no IPC channel to reach.
	 */
	onMount(() => {
		if (!isTauri()) return;
		void (async () => {
			try {
				const { invoke } = await import('@tauri-apps/api/core');
				await invoke('frontend_ready');
			} catch {
				// The watchdog in `lib.rs` covers this case.
			}
		})();
	});
</script>

{@render children()}

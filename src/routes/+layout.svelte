<script lang="ts">
	/**
	 * Root layout: the only place global CSS is imported, so every route inherits the theme
	 * tokens without importing them twice.
	 *
	 * It owns the two pieces of behaviour that belong to the app as a whole rather than to any
	 * one screen:
	 *
	 * 1. **Choosing which of the window's two roles to show**, from what the game client
	 *    reports. A game in progress means the build-order guide; the game ending means the
	 *    ordinary analysis view. This lives here, not in a page, because the app is a single
	 *    window whose *view* is swapped — so the swap has to happen above every route.
	 * 2. **Telling the shell the frontend has painted**, so it can reveal a window that is
	 *    deliberately declared hidden.
	 *
	 * The global hotkey is not handled here at all: it is a visibility control owned by the
	 * shell, and it does not change which view is shown.
	 */
	import { onMount } from 'svelte';
	import { get } from 'svelte/store';
	import { isTauri, pollClientApi, type LiveState } from '#lib/client-api';
	import { decideMode } from '#lib/game-mode';
	import type { WindowMode } from '#lib/window-mode';
	import { initWindowMode, revealWindowMode, windowMode } from '#lib/window-mode';
	import BuildOrderGuide from '#lib/components/BuildOrderGuide.svelte';
	import '../app.css';

	let { children } = $props();

	/**
	 * The mode the previous poll decided on, or `null` before the first one.
	 *
	 * Only a change here triggers a reveal, so a pinned or externally-forced mode cannot cause a
	 * reveal on every poll. See `poll`.
	 */
	let lastDecision: WindowMode | null = null;

	/** The latest client state. Drives the mode decision and the guide's status line. */
	let live = $state<LiveState>({
		status: 'idle',
		game: null,
		screens: null,
		lastOkAt: null,
		detail: null
	});

	/**
	 * Reads the client once and switches the window's view if the game state calls for it.
	 *
	 * The mode is applied only when it differs from what the shell reports, and the shell is the
	 * authority — the store mirrors it. Comparing against a local variable instead would miss a
	 * mode set from outside this component (the settings page, the shell's startup affordance),
	 * and the app would then re-apply a stale mode on every poll. The common case — a game
	 * running with the guide already up — therefore costs one comparison and no IPC.
	 */
	async function poll() {
		live = await pollClientApi();
		const mode = decideMode(live);

		// `null` means "cannot tell what is happening, leave the view alone" — see `decideMode`.
		if (!mode) return;

		// The transition, not the state, is what reveals the window: a match starting or ending is
		// the one moment the user asked the app to put itself on screen, and every poll after it
		// must do nothing so the app cannot fight a player who hid it mid-match.
		//
		// This tracks the last decision rather than comparing against the shell's reported mode.
		// Comparing against the shell looks more correct — the shell is the authority — but it
		// spins: anything that makes the shell report a mode this poll never asked for (a dev
		// affordance pinning the overlay, for instance) disagrees on every single poll, so the
		// window is re-revealed once a second and the hotkey appears to work only one way.
		if (mode === lastDecision) return;
		lastDecision = mode;

		if (mode !== get(windowMode)) {
			await revealWindowMode(mode);
		}
	}

	onMount(() => {
		// Returns the listener's teardown. Needed rather than optional: the shell reports a mode
		// that changed without the frontend being involved, so a leaked listener would keep
		// updating a store nothing is rendering.
		let stopListening: (() => void) | undefined;
		void initWindowMode().then((unlisten) => {
			stopListening = unlisten;
		});

		// Poll immediately, then once a second. The first read must not wait a second: a player
		// opening the app during a game should see the build guide straight away.
		void poll();
		const timer = setInterval(() => void poll(), 1000);

		// Tell the shell the frontend has painted, so it can reveal the window. The window is
		// declared `visible: false` so it cannot be seen empty, and this is what asks for it.
		// Failures are swallowed deliberately: the watchdog in `lib.rs` shows the window after
		// ten seconds regardless, and an unhandled rejection here would be worse than a late
		// window.
		void (async () => {
			if (!isTauri()) return;
			try {
				const { invoke } = await import('@tauri-apps/api/core');
				await invoke('frontend_ready');
			} catch {
				// The watchdog in `lib.rs` covers this case.
			}
		})();

		return () => {
			clearInterval(timer);
			stopListening?.();
		};
	});
</script>

{#if $windowMode === 'overlay'}
	<BuildOrderGuide />
{:else}
	{@render children()}
{/if}

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
	 * The global hotkey is not handled here, and that is deliberate: it decides its own mode in
	 * Rust (`game_watch::hotkey_mode`) rather than asking this page. A press has to work while the
	 * window is hidden, and a hidden WebKitGTK page has its timers suspended — so by the time the
	 * hotkey fires, this component is not running and could not answer. See ADR 0016.
	 */
	import { onMount } from 'svelte';
	import { get } from 'svelte/store';
	import { isTauri, pollClientApi, type LiveState } from '#lib/client-api';
	import { decideMode } from '#lib/game-mode';
	import type { WindowMode } from '#lib/window-mode';
	import { initWindowMode, revealWindowMode, setWindowMode, windowMode } from '#lib/window-mode';
	import { observeRepaints } from '#lib/repaint';
	import { initTheme, onThemeChange, themeHasSky, type ThemeId } from '#lib/theme';
	import BuildOrderGuide from '#lib/components/BuildOrderGuide.svelte';
	import WindowEdges from '#lib/components/WindowEdges.svelte';
	import '../app.css';

	let { children } = $props();

	/**
	 * The active theme, for deciding whether to render the ambient background layers.
	 *
	 * Only the celestial theme draws anything, and only in the analysis view — the overlay is a
	 * transparent window over a live game where stars would be unreadable noise.
	 */
	let theme = $state<ThemeId>('terran');

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

		// The first poll only *records* what the app should be, and never reveals the window.
		//
		// This is the difference between "the app was started" and "something happened in the
		// game". The watcher spawns the web view hidden at login and while the player is in the
		// menus, precisely so nothing appears until asked; revealing on the first poll undid that
		// one second after startup, and the symptom was the player pressing the hotkey for the
		// first time and watching the window *disappear*, because it had already come up on its
		// own. Measured through the player's own path: the window was visible before the
		// keypress.
		//
		// The mode is still applied, so the app is showing the right view if it is already on
		// screen — a player who opened it during a match must see the guide, and `setWindowMode`
		// changes the view without touching visibility.
		if (lastDecision === null) {
			lastDecision = mode;
			if (mode !== get(windowMode)) await setWindowMode(mode);
			return;
		}

		// Every poll after the first reveals only on a *transition*, so a match starting or
		// ending is the one moment the app puts itself on screen, and nothing afterwards can
		// fight a player who hid it mid-match.
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
		// Apply the stored theme before anything else on this mount. `app.html` already carries the
		// default so the first frame is never unthemed; this replaces it with the user's choice.
		void initTheme().then((chosen) => {
			theme = chosen;
		});
		// The CSS palette follows `data-theme` on its own, but the sky layer is markup, so this
		// component has to hear about a change made in Settings. Without it the ambient background
		// would only appear after a restart, which is exactly the kind of "it works if you relaunch
		// it" bug that reads as a caching problem rather than a missing listener.
		const stopTheme = onThemeChange((chosen) => {
			theme = chosen;
		});

		// Returns the listener's teardown. Needed rather than optional: the shell reports a mode
		// that changed without the frontend being involved, so a leaked listener would keep
		// updating a store nothing is rendering.
		let stopListening: (() => void) | undefined;
		void initWindowMode().then((unlisten) => {
			stopListening = unlisten;
		});

		// Keep the transparent view's backing store alive across repaints. This lives here, above
		// both modes, because the fault is not mode-specific: the guide collapsed to its status
		// line while the clock ticked, and the analysis view went blank outside the repainted
		// rectangle when it was interacted with. See `#lib/repaint` and ADR 0019.
		const stopRepainting = observeRepaints();

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
			stopTheme();
			stopRepainting();
		};
	});
</script>

{#if $windowMode === 'overlay'}
	<BuildOrderGuide />
{:else}
	<!--
		The analysis view sits on `.app-surface` and not on the page's own background. The window
		is declared `transparent: true` for the overlay's sake, and in that state WebKitGTK does
		not paint a `background-color` set on `html`/`body`; a full-size element's background *is*
		painted. Without this wrapper the analysis view is a see-through pane over the desktop.
		See the `.app-surface` comment in app.css for the measurements.
	-->
	<div class="app-surface h-full w-full">
		<!--
			The ambient background sits UNDER the page content: `position: fixed` so it covers the
			window while the surface scrolls, and `pointer-events: none` so it never takes a click.
			Only the Space theme renders it. See `.theme-sky` in app.css.
		-->
		<!--
			The window's own resize border and top-edge drag strip. Rendered here, above the page and
			OUTSIDE `.app-surface`, because its handles are `position: fixed` against the viewport: a
			window edge must not scroll with the content, and it must exist on every page rather than
			only where the title bar happens to be. Overlay mode deliberately does not get one — that
			window is a 380x600 panel the app positions itself (ADR 0016).
		-->
		<WindowEdges />
		{#if themeHasSky(theme)}
			<div class="theme-nebulae" aria-hidden="true"></div>
			<div class="theme-sky" aria-hidden="true"></div>
		{/if}
		<div class="relative">
			{@render children()}
		</div>
	</div>
{/if}

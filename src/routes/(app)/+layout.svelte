<script lang="ts">
	/**
	 * Chrome for the analysis app: navigation, and a live indicator for the game clock.
	 *
	 * The indicator is here rather than on a settings page because whether the client API
	 * is reachable decides whether the overlay can follow a real game at all — it is the
	 * project's single biggest unknown, so it is surfaced permanently.
	 *
	 * The window's title bar is NOT here: it lives in the root layout, outside the scrolling
	 * area, because the scrollbar has to begin below it (see `.app-surface` in app.css) and
	 * because it must survive route changes.
	 */
	import { onDestroy } from 'svelte';
	import { page } from '$app/state';
	import { isTauri, pollClientApi, type LiveState } from '#lib/client-api';

	let { children } = $props();

	let live = $state<LiveState>({
		status: 'idle',
		game: null,
		screens: null,
		lastOkAt: null,
		detail: null
	});

	const timer = setInterval(async () => {
		live = await pollClientApi();
	}, 2000);

	void (async () => {
		live = await pollClientApi();
	})();

	onDestroy(() => clearInterval(timer));

	const nav = [
		{ href: '/matches', label: 'Matches' },
		{ href: '/builds', label: 'Builds' },
		{ href: '/settings', label: 'Settings' }
	];

	/**
	 * A dot colour and label for the client-API state.
	 *
	 * Outside the desktop shell the warning colour is wrong: a browser cannot read loopback,
	 * so `unavailable` there is a limitation of the host rather than a problem the user can
	 * fix, and it is deliberately not dressed up as a fault.
	 */
	const indicator = $derived.by(() => {
		const inBrowser = !isTauri();
		switch (live.status) {
			case 'live':
				return { colour: 'bg-ahead', text: 'Game running' };
			case 'replay':
				return { colour: 'bg-race-protoss', text: 'Replay open' };
			case 'unavailable':
				return inBrowser
					? { colour: 'bg-accent-2/70', text: 'Desktop app only' }
					: { colour: 'bg-behind', text: 'Game not detected' };
			case 'error':
				return { colour: 'bg-behind', text: 'Client API error' };
			default:
				return { colour: 'bg-accent-2/70', text: 'Checking…' };
		}
	});
</script>

<div class="flex min-h-full flex-col">
	<!--
		The header's bottom rule is the theme's SECONDARY colour, not a neutral hairline. Every screen
		in the app is under this bar, so it is the one border the user cannot avoid seeing, and leaving
		it neutral was the single biggest reason the palettes read as single-colour.
	-->
	<header
		class="border-b bg-surface-900/70"
		style="border-bottom-color: color-mix(in oklab, var(--color-accent-2) 38%, transparent)"
	>
		<div class="mx-auto flex max-w-6xl items-center gap-6 px-6 py-3">
			<a href="/" class="flex items-center gap-2 text-sm font-semibold tracking-tight">
				<!--
					A thin accent rule in front of the wordmark, echoing the console's light strip. It
					takes its colour from the theme, so the nav identifies the theme at a glance.
				-->
				<span class="h-4 w-[3px] rounded-full bg-accent" aria-hidden="true"></span>
				<!--
					A second, shorter bar in the secondary colour. The pair is what makes the wordmark two-tone
					rather than "a title with a coloured tick": on every screen the primary hue sits directly
					beside the secondary one, which is the cheapest possible statement of the scheme.
				-->
				<span class="h-2.5 w-[2px] rounded-full bg-accent-2" aria-hidden="true"></span>
				SC2 Science Vessel
			</a>
			<nav class="flex gap-1" aria-label="Main">
				{#each nav as item (item.href)}
					{@const active = page.url.pathname === item.href}
					<a
						href={item.href}
						aria-current={active ? 'page' : undefined}
						class="rounded px-3 py-1.5 text-sm {active ? 'nav-active' : 'nav-idle'}"
					>
						{item.label}
					</a>
				{/each}
			</nav>
			<div class="ml-auto flex items-center gap-2" title={live.detail ?? undefined}>
				<span class="h-2 w-2 rounded-full {indicator.colour}" aria-hidden="true"></span>
				<span class="text-xs text-ink-500" role="status">{indicator.text}</span>
			</div>
		</div>
	</header>

	<main class="mx-auto w-full max-w-6xl flex-1 px-6 py-8">
		{@render children()}
	</main>

	<footer
		class="border-t px-6 py-3"
		style="border-top-color: color-mix(in oklab, var(--color-accent-2) 30%, transparent)"
	>
		<p class="mx-auto max-w-6xl text-xs text-ink-500">
			Runs entirely on this machine. No account, no upload, no telemetry. The only network
			request the app can make is to the game client on localhost.
		</p>
	</footer>
</div>

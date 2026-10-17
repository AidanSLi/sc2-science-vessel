<script lang="ts">
	/**
	 * Chrome for the analysis app: navigation, and a live indicator for the game clock.
	 *
	 * The indicator is here rather than on a settings page because whether the client API
	 * is reachable decides whether the overlay can follow a real game at all — it is the
	 * project's single biggest unknown, so it is surfaced permanently.
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
					? { colour: 'bg-surface-700', text: 'Desktop app only' }
					: { colour: 'bg-behind', text: 'Game not detected' };
			case 'error':
				return { colour: 'bg-behind', text: 'Client API error' };
			default:
				return { colour: 'bg-surface-700', text: 'Checking…' };
		}
	});
</script>

<div class="flex min-h-full flex-col">
	<header class="border-b border-surface-800 bg-surface-900/70">
		<div class="mx-auto flex max-w-6xl items-center gap-6 px-6 py-3">
			<a href="/" class="text-sm font-semibold tracking-tight">SC2 Science Vessel</a>
			<nav class="flex gap-1" aria-label="Main">
				{#each nav as item (item.href)}
					{@const active = page.url.pathname === item.href}
					<a
						href={item.href}
						aria-current={active ? 'page' : undefined}
						class="rounded px-3 py-1.5 text-sm {active
							? 'bg-surface-800 text-ink-100'
							: 'text-ink-500 hover:bg-surface-850 hover:text-ink-300'}"
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

	<footer class="border-t border-surface-800 px-6 py-3">
		<p class="mx-auto max-w-6xl text-xs text-ink-500">
			Runs entirely on this machine. No account, no upload, no telemetry. The only network
			request the app can make is to the game client on localhost.
		</p>
	</footer>
</div>

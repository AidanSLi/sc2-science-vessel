<script lang="ts">
	/**
	 * Build-order library and the launcher for the overlay.
	 *
	 * The one sample build is shown with its steps so the overlay has something honest to
	 * follow. Importing builds (SpawningTool snapshot, SALT strings) is Phase 4 and the UI
	 * says so rather than offering dead controls.
	 */
	import { onMount } from 'svelte';
	import { advance } from '#lib/builds';
	import { buildsForMatchup, bundledSnapshotDate, bundledSnapshotSource } from '#lib/build-library';
	import { formatGameTime } from '#lib/clock';
	import { DEFAULT_OVERLAY_SHORTCUT, loadSettings } from '#lib/settings';
	import { sampleBuildOrder } from '#lib/sample-build';
	import type { BuildOrder } from '#lib/types';

	let build = $state(sampleBuildOrder);

	/**
	 * Builds for the sample's matchup, exact matchup first and wildcards second.
	 *
	 * The library is the whole point of the build-order half of the app, so the page lists the
	 * real bundled snapshot rather than only the hand-written sample. It is read from the
	 * inlined `src/lib/builds.json` (see `build-library.ts`), so this page works with no network.
	 */
	const library = $derived(buildsForMatchup(build.matchup));

	/**
	 * Switches the previewed build and re-anchors the preview clock.
	 *
	 * @param next The build to show.
	 */
	function selectBuild(next: BuildOrder) {
		build = next;
	}

	/**
	 * The registered overlay key.
	 *
	 * Read from settings rather than hardcoded: the key is user-configurable, and the previous
	 * hardcoded `Ctrl+Alt+B` was already wrong by the time it was written — that combination is
	 * taken on at least one real KDE session, which is why the default is `Ctrl+Alt+O`. A stale
	 * string here would send a player to a key that does nothing.
	 */
	let overlayShortcut = $state(DEFAULT_OVERLAY_SHORTCUT);

	/** The overlay is a separate window; this is the in-app way to open it. */
	let overlayMessage = $state<string | null>(null);

	/**
	 * Opens the build-order overlay window through the Rust command.
	 *
	 * This deliberately delegates to `toggle_overlay` rather than constructing a
	 * `WebviewWindow` here. The overlay's whole behaviour is in how the window is
	 * *configured* — borderless, always-on-top, click-through, out of the taskbar — and the
	 * Rust side re-applies those flags on every show, partly because they do not reliably
	 * survive a hide/show cycle. A window built from the frontend with
	 * `new WebviewWindow('overlay', …)` looks right for a moment and then behaves like an
	 * ordinary window, which the player would only find out mid-game when the overlay started
	 * eating their mouse clicks. One construction path also means one place to change when
	 * the overlay becomes a native layer-shell surface (ADR 0003).
	 *
	 * Failure is expected in a browser tab (there is no desktop shell there), so the message
	 * distinguishes "no desktop shell" from a real error.
	 */
	async function openOverlay() {
		overlayMessage = null;
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			await invoke('toggle_overlay');
			overlayMessage = `Overlay shown. ${overlayShortcut} toggles it.`;
		} catch (err) {
			overlayMessage = `Needs the desktop app: ${
				err instanceof Error ? err.message : String(err)
			}`;
		}
	}

	const total = $derived(build.steps.length);
	const lastAt = $derived(build.steps[total - 1]?.timeInGameSeconds ?? 0);
	// A build previewed at a fixed clock, so the list shows the styling of a past step.
	const preview = $derived(advance(build.steps, 120));

	/**
	 * Loads the registered shortcut once the page mounts.
	 *
	 * In a browser there are no settings to read, so the default stands and the message below the
	 * button is about the desktop shell rather than a wrong key.
	 */
	onMount(async () => {
		try {
			overlayShortcut = (await loadSettings()).overlayShortcut;
		} catch {
			// Keep the default; the shortcut is only shown as a hint.
		}
	});
</script>

<section class="space-y-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold">Builds</h1>
			<p class="tabular mt-1 text-sm text-ink-500">
				{total} steps · ends at {formatGameTime(lastAt)} game time · preset for {build.matchup}
			</p>
		</div>
		<button
			type="button"
			class="rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-300 hover:bg-surface-700"
			onclick={openOverlay}
		>
			Open overlay
		</button>
	</div>

	{#if overlayMessage}
		<p class="rounded border border-surface-800 bg-surface-900/60 px-3 py-2 text-sm text-ink-300">
			{overlayMessage}
		</p>
	{/if}

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">{build.title}</h2>
		<p class="mt-1 text-xs text-ink-500">
			{#if build.source === 'bundled'}
				From the bundled SpawningTool snapshot ({bundledSnapshotDate.slice(0, 10)}). Guide times
				are game-clock seconds, so the overlay can follow them directly.
			{:else}
				Sample fixture shipped with the skeleton.
			{/if}
		</p>

		<ol class="mt-4 space-y-px">
			{#each build.steps as step, index (index)}
				{@const isUpcoming = index === preview.nextIndex}
				<li
					class="tabular flex items-center gap-3 rounded px-2 py-1 text-sm {isUpcoming
						? 'bg-surface-800 text-ink-100'
						: 'text-ink-300'}"
				>
					<span class="w-10 text-right text-xs text-ink-500">s{step.supply}</span>
					<span class="w-12 text-xs text-ink-500">{formatGameTime(step.timeInGameSeconds)}</span>
					<span class="flex-1">{step.name}</span>
					{#if step.note}
						<span class="text-xs text-ink-500">{step.note}</span>
					{/if}
					{#if step.isChronoboosted}
						<span class="text-xs text-race-protoss">chrono</span>
					{/if}
				</li>
			{/each}
		</ol>
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">{library.length} {build.matchup} build{library.length === 1 ? '' : 's'} in the library</h2>
		<p class="mt-1 text-xs text-ink-500">
			Matchup exact matches first, then builds written against any opponent. Click one to
			preview its steps above.
		</p>

		<ul class="mt-4 grid gap-1 sm:grid-cols-2">
			{#each library as candidate (candidate.id)}
				<li>
					<button
						type="button"
						class="w-full rounded px-2 py-1.5 text-left text-sm {candidate.id === build.id
							? 'bg-surface-800 text-ink-100'
							: 'text-ink-300 hover:bg-surface-850'}"
						onclick={() => selectBuild(candidate)}
					>
						<span class="block truncate">{candidate.title}</span>
						<span class="text-xs text-ink-500">
							{candidate.steps.length} steps · ends
							{formatGameTime(candidate.steps[candidate.steps.length - 1]?.timeInGameSeconds ?? 0)}
						</span>
					</button>
				</li>
			{/each}
		</ul>
	</div>

	<div class="rounded-lg border border-dashed border-surface-700 p-4">
		<h2 class="text-sm font-semibold">Refreshing the library</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			The snapshot above is committed, so the app works offline. Refreshing it is the only part
			of the app that uses the network, and it is an explicit command rather than something the
			app does on its own:
		</p>
		<pre class="mt-3 overflow-x-auto rounded bg-surface-950 p-3 text-xs text-ink-300"><code>python tools/refresh_build_snapshot.py --out src/lib/builds.json --pages 3</code></pre>
		<p class="mt-2 max-w-2xl text-xs text-ink-500">
			Source: {bundledSnapshotSource}. If the refresh fails it keeps the existing snapshot
			rather than writing an empty one, because an empty library looks current but is not.
		</p>
	</div>
</section>

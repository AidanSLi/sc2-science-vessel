<script lang="ts">
	/**
	 * Build-order library and the launcher for the overlay.
	 *
	 * The one sample build is shown with its steps so the overlay has something honest to
	 * follow. Importing builds (SpawningTool snapshot, SALT strings) is Phase 4 and the UI
	 * says so rather than offering dead controls.
	 */
	import { advance } from '#lib/builds';
	import { formatGameTime } from '#lib/clock';
	import { sampleBuildOrder } from '#lib/sample-build';

	let build = $state(sampleBuildOrder);

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
			overlayMessage = 'Overlay shown. Ctrl+Alt+B toggles it.';
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
			Sample fixture shipped with the skeleton. Replaced by parsed and imported builds in
			Phases 1 and 4.
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
					{#if step.isChronoboosted}
						<span class="text-xs text-race-protoss">chrono</span>
					{/if}
				</li>
			{/each}
		</ol>
	</div>

	<div class="rounded-lg border border-dashed border-surface-700 p-4">
		<h2 class="text-sm font-semibold">Importing builds</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			Two sources are planned. A bundled snapshot of SpawningTool's library, refreshed
			explicitly by the user so the app stays offline. And pasted SALT strings, which need no
			network at all — but note the format's reference implementation is decode-only, so an
			encoder has to be written before builds can be exported.
		</p>
	</div>
</section>

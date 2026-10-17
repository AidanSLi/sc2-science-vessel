<script lang="ts">
	/**
	 * Landing page for the analysis app.
	 *
	 * It reports the project's state honestly rather than showing invented statistics: the
	 * skeleton has no data pipeline yet, and a dashboard of zeros would look like a working
	 * app that found nothing.
	 */
	import { formatGameTime } from '#lib/clock';
	import { sampleBuildOrder } from '#lib/sample-build';

	/**
	 * Milestones, with what "done" means for each.
	 *
	 * Kept as data rather than prose so the count above cannot drift out of step with the list.
	 * A milestone is done when it works against a real replay or a running game, not when the
	 * code exists — several of these were marked done only after an end-to-end run.
	 */
	const milestones = [
		{ label: 'Skeleton (SvelteKit + Tailwind + Tauri 2)', done: true },
		{ label: 'Confirm the client API clock on localhost:6119', done: true },
		{ label: 'Python sidecar: index and parse a replay library', done: true },
		{ label: 'Metric layer: economy, military, production, army', done: true },
		{ label: 'Overlay driven by the game clock, per-matchup presets', done: true },
		{ label: 'Build imports: SpawningTool snapshot and SALT', done: true },
		{ label: 'Resident watcher that owns the window lifetime', done: true },
		{ label: 'Overlay over a fullscreen game (needs a compositor surface)', done: false },
		{ label: 'Windows overlay and game detection', done: false }
	];

	const remaining = milestones.filter((m) => !m.done).length;
	const buildSummary = `${sampleBuildOrder.steps.length} steps for ${sampleBuildOrder.matchup}, ending at ${formatGameTime(
		sampleBuildOrder.steps[sampleBuildOrder.steps.length - 1].timeInGameSeconds
	)}`;
</script>

<section class="space-y-8">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">SC2 Science Vessel</h1>
		<p class="mt-2 max-w-3xl text-sm text-ink-300">
			Replay analysis and a build-order coach for StarCraft II, running entirely on this
			machine. No account, no upload, and no network beyond the game client on loopback.
		</p>
	</div>

	<div class="grid gap-4 md:grid-cols-3">
		<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
			<h2 class="text-sm font-semibold">Analysis</h2>
			<p class="mt-1 text-xs text-ink-500">
				Worker, economy, production and battle metrics from your own replays.
			</p>
		</div>
		<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
			<h2 class="text-sm font-semibold">Build orders</h2>
			<p class="tabular mt-1 text-xs text-ink-500">{buildSummary}</p>
		</div>
		<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
			<h2 class="text-sm font-semibold">Overlay</h2>
			<p class="mt-1 text-xs text-ink-500">
				Always on top, click-through, following the game's own clock.
			</p>
		</div>
	</div>

	<div class="rounded-lg border border-surface-800 p-4">
		<h2 class="text-sm font-semibold">
			Milestones <span class="font-normal text-ink-500">· {remaining} remaining</span>
		</h2>
		<ul class="mt-3 space-y-1.5">
			{#each milestones as m (m.label)}
				<li class="flex items-center gap-3 text-sm">
					<span
						class="inline-block h-1.5 w-1.5 shrink-0 rounded-full {m.done
							? 'bg-ahead'
							: 'bg-surface-700'}"
						aria-hidden="true"
					></span>
					<span class={m.done ? 'text-ink-100' : 'text-ink-500'}>{m.label}</span>
				</li>
			{/each}
		</ul>
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-6">
		<h2 class="text-sm font-semibold">What this build is</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			A beta: replay analysis, a build-order library and a live overlay, in one window that
			changes shape with what you are doing. The app waits for StarCraft&nbsp;II in the
			background and only pays for its web view while the game is open. The feasibility study
			behind the design, including the numbers it was verified against, is in
			<code class="rounded bg-surface-950 px-1 py-0.5 text-xs">docs/assessment.md</code>.
		</p>
	</div>
</section>

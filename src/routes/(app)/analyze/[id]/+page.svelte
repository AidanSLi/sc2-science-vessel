<script lang="ts">
	/**
	 * Detail view for one game. The skeleton shows the shape the real analysis will take
	 * and states plainly what is not wired up yet, so the empty panels are not mistaken
	 * for missing data.
	 */
	import { formatGameTime, gameToRealSeconds } from '#lib/clock';
	import type { MetricGroup } from '#lib/types';

	/** The group tabs, with what each will eventually plot. */
	const groups: { id: MetricGroup; label: string; will: string }[] = [
		{ id: 'economy', label: 'Economy', will: 'workers, income, resources banked' },
		{ id: 'military', label: 'Military', will: 'trades, resources lost and killed' },
		{ id: 'production', label: 'Production', will: 'structure uptime, supply blocks' },
		{ id: 'army', label: 'Army', will: 'composition, expansions, tech timing' }
	];

	let selected = $state<MetricGroup>('economy');

	/** Placeholder values, only to show the layout. Not from a replay. */
	const placeholder = {
		map: 'Acropolis LE',
		matchup: 'ZvT',
		lengthInGameSeconds: 1194,
		version: '5.0.16.97364'
	};

	const group = $derived(groups.find((g) => g.id === selected) ?? groups[0]);
</script>

<section class="space-y-6">
	<div>
		<h1 class="text-xl font-semibold">{placeholder.map}</h1>
		<p class="tabular mt-1 text-sm text-ink-500">
			{placeholder.matchup} · {formatGameTime(placeholder.lengthInGameSeconds)} game time
			({Math.round(gameToRealSeconds(placeholder.lengthInGameSeconds) / 60)} min wall clock at
			Faster) · patch {placeholder.version}
		</p>
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<p class="text-sm text-ink-300">
			No analysis yet. Metric computation arrives with the Python sidecar (Phase 1 of
			<a class="underline decoration-dotted" href="/">docs/assessment.md</a>); this view is the
			layout it will fill.
		</p>
	</div>

	<div>
		<div class="flex flex-wrap gap-1" role="tablist" aria-label="Metric groups">
			{#each groups as g (g.id)}
				<button
					type="button"
					role="tab"
					aria-selected={selected === g.id}
					class="rounded px-3 py-1.5 text-sm {selected === g.id
						? 'bg-surface-800 text-ink-100'
						: 'text-ink-500 hover:bg-surface-900'}"
					onclick={() => (selected = g.id)}
				>
					{g.label}
				</button>
			{/each}
		</div>

		<div
			class="mt-3 flex h-64 items-center justify-center rounded-lg border border-dashed border-surface-700 bg-surface-900/40"
		>
			<p class="text-sm text-ink-500">
				{group.label} chart area — will plot {group.will}.
			</p>
		</div>
	</div>

	<div class="grid gap-4 md:grid-cols-2">
		<div class="rounded-lg border border-surface-800 p-4">
			<h2 class="text-sm font-semibold">Build order</h2>
			<p class="mt-1 text-xs text-ink-500">
				Parsed steps with ±10 s discrepancy markers against the selected reference build.
			</p>
		</div>
		<div class="rounded-lg border border-surface-800 p-4">
			<h2 class="text-sm font-semibold">Battle timeline</h2>
			<p class="mt-1 text-xs text-ink-500">
				Engagements with net resource swing; hovering one will mark where it happened on the map.
			</p>
		</div>
	</div>
</section>

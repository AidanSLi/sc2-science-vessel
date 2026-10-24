<script lang="ts">
	/**
	 * The build-order panel: what the player actually built, and how it compares to a reference.
	 *
	 * This is the feature the list asks for as "select which build you were trying to do after
	 * finishing each game, or compare to the closest of some set builds, and note discrepancies
	 * of +/- 10 seconds". Three things follow from that wording:
	 *
	 * 1. A reference is chosen by the user, but a good one is *ranked first* so the common case
	 *    (analyse now, read later) needs no decision. The ranking is on the bundled SpawningTool
	 *    snapshot, which is offline (ADR 0002).
	 * 2. Discrepancies are shown per step rather than as one score, because "+1 Gateway 14 s late"
	 *    is actionable and "72% match" is not. The score is shown too, but as a summary.
	 * 3. The player's own build order is shown in full beside the comparison. A comparison alone
	 *    cannot tell you what you *did* build when the answer is "something else entirely", and
	 *    the step list is what SpawningTool itself is for.
	 *
	 * No judgement is offered about whether the build was good, because the replay does not
	 * contain the opponent's intent or the game state that would justify one.
	 */
	import TimelineChart from './TimelineChart.svelte';
	import { formatGameTime } from '#lib/clock';
	import { buildsForMatchup, referenceSteps } from '#lib/build-library';
	import { playerStyle, rankBuilds } from '#lib/analysis-view';
	import type { AnalysisPlayer } from '#lib/analysis';

	interface Props {
		/** The player whose build is being examined. */
		player: AnalysisPlayer;
		/** The game's matchup, e.g. `ZvT`, used to pick candidate references. */
		matchup: string;
		/** The game's length, for the shared x-axis. */
		lengthSeconds: number;
		/** Every player, so a mirror matchup still colours this one distinctly. */
		allPlayers: readonly Pick<AnalysisPlayer, 'pid' | 'race'>[];
		/** The second the page is highlighting, or null. */
		cursorSecond?: number | null;
		/** Reports a pointer position on the progress chart, in game seconds. */
		onhover?: (second: number | null) => void;
	}

	let { player, matchup, lengthSeconds, allPlayers, cursorSecond = null, onhover }: Props = $props();

	/** The bundled references for this matchup, exact matchups before `vX` wildcards. */
	const candidates = $derived(buildsForMatchup(matchup));

	/** Every candidate, scored and sorted best first. */
	const ranked = $derived(
		rankBuilds(
			player.buildOrder,
			candidates.map((build) => ({ title: build.title, steps: referenceSteps(build) }))
		)
	);

	/**
	 * Which ranked candidate the user picked, or -1 for "just show my build".
	 *
	 * Stored as an index into `ranked` rather than a build id because the ranking is what names
	 * the options: "the closest match" is the useful first entry even when the snapshot holds
	 * thirty builds for the matchup, and re-deriving which build that was from an id would mean
	 * the option list and the label could disagree.
	 */
	let chosen = $state(0);

	/** The comparison to render, or null when the user asked for no reference. */
	const selected = $derived(chosen >= 0 ? (ranked[chosen] ?? null) : null);

	/** This player's colour, and whether their line should be dashed in a mirror matchup. */
	const style = $derived(playerStyle(player, allPlayers));

	/**
	 * The build-progress chart: step count over time for both the player and the reference.
	 *
	 * Cumulative step index rather than supply or step name, because it is the only series that
	 * is meaningful for two different builds on one axis — a gap between the lines *is* the
	 * drift, widening as the game goes on, which is exactly the shape a reader wants to see.
	 * A reference the player never followed drifts away and stops rising; that is a correct and
	 * informative picture, not a rendering bug.
	 */
	const progressSeries = $derived.by(() => {
		const series = [
			{
				label: `${player.name} (${player.buildOrder.length} steps)`,
				colorToken: style.colorToken,
				x: player.buildOrder.map((step) => step.second),
				y: player.buildOrder.map((_, index) => index + 1),
				stepped: true
			}
		];
		if (selected && selected.name) {
			const reference = candidates.find((build) => build.title === selected.name);
			if (reference) {
				series.push({
					label: reference.title,
					colorToken: 'neutral',
					x: reference.steps.map((step) => step.timeInGameSeconds),
					y: reference.steps.map((_, index) => index + 1),
					stepped: true
				});
			}
		}
		return series;
	});

	/** Steps the player hit within tolerance, on time or not. */
	const onTarget = $derived(selected?.matched.filter((entry) => entry.verdict === 'on time').length ?? 0);

	/** Steps the player hit but late or early. */
	const offTarget = $derived((selected?.matched.length ?? 0) - onTarget);

	/**
	 * Formats a delta as a signed offset.
	 *
	 * @param seconds The difference between the actual and planned second.
	 * @returns A string such as `+14.0s`.
	 */
	function delta(seconds: number): string {
		const rounded = seconds.toFixed(1);
		return seconds > 0 ? `+${rounded}s` : `${rounded}s`;
	}
</script>

<section class="space-y-4">
	<header class="flex flex-wrap items-baseline justify-between gap-2">
		<h2 class="section-heading text-sm font-semibold">Build order</h2>
		<p class="text-xs text-ink-500">
			{player.buildOrder.length} steps tracked · matching tolerance ±10 s
		</p>
	</header>

	{#if player.buildOrder.length === 0}
		<p class="rounded-lg border border-dashed border-edge/45 bg-surface-900/40 p-4 text-sm text-ink-500">
			No build order could be recovered for {player.name}. The tracker stream in this replay has no
			build events for them.
		</p>
	{:else}
		<TimelineChart
			title="Build progress"
			series={progressSeries}
			{lengthSeconds}
			secondFormatter={formatGameTime}
			{cursorSecond}
			{onhover}
		/>

		<div class="grid gap-4 lg:grid-cols-2">
			<div class="space-y-2">
				<label class="block text-xs font-semibold tracking-wide text-ink-300 uppercase" for="reference">
					Compare against
				</label>
				<select
					id="reference"
					class="w-full rounded border border-edge/60 bg-surface-900 px-2 py-1.5 text-sm text-ink-100"
					bind:value={chosen}
				>
					{#each ranked as entry, index (entry.name + index)}
						<option value={index}>
							{entry.similarity === 1 ? 'All steps matched' : `${Math.round(entry.similarity * 100)}%`}
							— {entry.name}
						</option>
					{/each}
					<option value={-1}>My build only, no comparison</option>
				</select>

				{#if selected}
					<div class="panel p-3">
						<p class="tabular text-sm text-ink-100">
							{Math.round(selected.similarity * 100)}% of the plan
							<span class="text-ink-500">
								({onTarget} on time, {offTarget} off, {selected.missing.length} never made)
							</span>
						</p>
						<p class="mt-1 text-xs text-ink-500">
							Average drift on matched steps
							{selected.matched.length > 0
								? `${(selected.totalDriftSeconds / selected.matched.length).toFixed(1)} s`
								: '—'}
							· measured against the closest of {candidates.length} bundled
							{matchup || 'matching'} builds.
						</p>
					</div>
				{/if}

				{#if selected && selected.matched.length > 0}
					<ul class="max-h-64 space-y-1 overflow-y-auto pr-1">
						{#each selected.matched as entry (entry.name + entry.expectedSecond)}
							<li class="rounded bg-surface-900/40 px-2 py-1 text-xs">
								<div class="flex items-baseline justify-between gap-2">
									<span class="truncate text-ink-300">{entry.name}</span>
									<span class="tabular shrink-0 {entry.verdict === 'on time' ? 'text-ink-500' : entry.verdict === 'late' ? 'text-behind' : 'text-ahead'}">
										{formatGameTime(entry.actualSecond)}
										<span class="text-ink-500">vs {formatGameTime(entry.expectedSecond)}</span>
										{entry.verdict === 'on time' ? 'on time' : delta(entry.deltaSeconds)}
									</span>
								</div>
								{#if entry.note}
									<!--
										The plan author's note, shown against the step the player actually hit. For
										the steps named `Action` the note is the whole instruction, so a comparison
										that withheld it could report `Action on time` while telling the reader nothing
										about what they were supposed to do.
									-->
									<p class="mt-0.5 text-ink-500">{entry.note}</p>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div class="space-y-2">
				<h3 class="text-xs font-semibold tracking-wide text-ink-300 uppercase">
					{matchup || 'This game'}
				</h3>
				<ol class="max-h-96 space-y-1 overflow-y-auto pr-1">
					{#each player.buildOrder as step, index (step.second + step.name + index)}
						<li class="flex items-baseline gap-2 font-mono text-xs">
							<span class="tabular w-12 shrink-0 text-right text-ink-500">{formatGameTime(step.second)}</span>
							<span class="tabular w-6 shrink-0 text-right text-ink-500">{step.supply ?? ''}</span>
							<span class="min-w-0 text-ink-300">{step.name}</span>
						</li>
					{/each}
				</ol>
				{#if selected && selected.missing.length > 0}
					<div class="rounded border border-edge/55 bg-surface-900/40 p-2">
						<p class="text-xs text-ink-500">Planned but never seen in this game</p>
						<ul class="mt-1 space-y-0.5">
							{#each selected.missing.slice(0, 12) as entry (entry.name + entry.expectedSecond)}
								<li class="text-xs">
									<span class="text-ink-300">{entry.name}</span>
									<span class="tabular text-ink-500">at {formatGameTime(entry.expectedSecond)}</span>
									{#if entry.note}<span class="text-ink-500">— {entry.note}</span>{/if}
								</li>
							{/each}
						</ul>
						{#if selected.missing.length > 12}
							<p class="mt-1 text-xs text-ink-500">+{selected.missing.length - 12} more</p>
						{/if}
					</div>
				{/if}
			</div>
		</div>
	{/if}
</section>

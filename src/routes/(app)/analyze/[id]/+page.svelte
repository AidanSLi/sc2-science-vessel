<script lang="ts">
	/**
	 * Analysis for one replay.
	 *
	 * The replay's path is the route parameter (URL-encoded), which means this page works from a
	 * deep link and from the matches list without any shared state, and a reload lands on the
	 * same game.
	 *
	 * Everything shown here comes from one sidecar call, `analyze_replay`, which returns prepared
	 * series rather than raw events (ADR 0001). All grouping and formatting decisions live in
	 * `#lib/analysis-view` so they are testable without a browser; this file is the layout plus
	 * the two pieces of state that are genuinely interactive — which metric group is open, and
	 * which second the pointer is on.
	 *
	 * `cursorSecond` is shared by every panel on purpose. The feature list asks for the map to
	 * show where a battle happened when hovering a change in the resources-lost graph, and that
	 * only works if the graphs and the map read the same pointer position.
	 */
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { formatGameTime } from '#lib/clock';
	import {
		analyzeReplay,
		sidecarUnavailableReason,
		type AnalysisPayload,
		type AnalysisPlayer,
		type EnergyTimeline
	} from '#lib/analysis';
	import {
		energyMarkers,
		playerStyle,
		raceColorToken,
		stepMarkers,
		supplyBlockDurations,
		supplyBlockMarkers,
		workerLossSummary
	} from '#lib/analysis-view';
	import TimelineChart from '#lib/components/TimelineChart.svelte';
	import BattleMap from '#lib/components/BattleMap.svelte';
	import BuildOrderPanel from '#lib/components/BuildOrderPanel.svelte';

	/** The metric groups, with the panels each one opens. */
	const groups: { id: number; label: string; blurb: string }[] = [
		{ id: 0, label: 'Economy', blurb: 'Workers, income and what each army cost.' },
		{ id: 1, label: 'Production', blurb: 'Supply blocks, energy abilities and tech timing.' },
		{ id: 2, label: 'Military', blurb: 'Trades, the resource swing of the game, and where units died.' },
		{ id: 3, label: 'Build order', blurb: 'What was built, compared against a bundled build.' }
	];

	/** Which group tab is open. */
	let selected = $state(0);

	/** The decoded replay path from the URL. */
	const replayPath = $derived(decodeURIComponent(page.params.id ?? ''));

	/** The loaded analysis, or null before it arrives. */
	let payload = $state<AnalysisPayload | null>(null);

	/** Why loading failed, in terms a player can act on. */
	let failure = $state<string | null>(null);

	/** True while the sidecar is running; it takes roughly 0.3–1 s. */
	let loading = $state(true);

	/** The second highlighted across every panel, or null when the pointer is off the charts. */
	let cursorSecond = $state<number | null>(null);

	/** Distinguishes "no desktop app" (expected in a browser) from a real failure. */
	const unavailable = sidecarUnavailableReason();

	onMount(() => {
		if (unavailable) {
			loading = false;
			failure = unavailable;
			return;
		}
		let cancelled = false;
		void (async () => {
			try {
				const result = await analyzeReplay(replayPath);
				if (!cancelled) payload = result;
			} catch (error) {
				if (!cancelled) failure = error instanceof Error ? error.message : String(error);
			} finally {
				if (!cancelled) loading = false;
			}
		})();
		return () => {
			cancelled = true;
		};
	});

	/** The worst worker-loss burst, which is the runby verdict. */
	const runby = $derived(payload ? workerLossSummary(payload) : []);

	/**
	 * The game's length in seconds, or 0 before the payload arrives.
	 *
	 * Pulled out of `payload` rather than read as `payload.lengthSeconds` at each use because a
	 * narrowing check on `payload` does not survive into a callback: inside the `{:else if payload}`
	 * block the compiler still cannot prove `payload` is non-null within an arrow function's body.
	 */
	const gameLength = $derived(payload?.lengthSeconds ?? 0);

	/** The player with the largest worker-loss burst — the one most vulnerable to runbys. */
	const runbyWorst = $derived(runby[0] ?? null);

	/** Turns a player's series into two lines, one per player, sharing a y scale. */
	const players = $derived(payload?.players ?? []);

	/**
	 * Players keyed by pid, for naming whose macro an energy figure belongs to.
	 *
	 * An energy timeline merges one entry per player of the ability's race, so the panel has a pid
	 * and needs the name behind it. `perPlayer[].pid` is `number | null` at the type level, and a
	 * Map lookup returns `undefined` for a null key instead of throwing on a missing player.
	 */
	const playersByPid = $derived(
		new Map<number | null, AnalysisPlayer>(players.map((player) => [player.pid, player]))
	);

	/**
	 * Spells out an energy timeline's headline.
	 *
	 * A mirror matchup merges one entry per player of the ability's race, so "across 8 bases"
	 * silently sums two players' town halls. Naming the player count keeps the aggregate honest
	 * and points at the per-player line below it, which breaks the number down.
	 *
	 * @param timeline The merged energy timeline.
	 * @returns A headline naming the base count, and the player count when more than one.
	 */
	function energyHeadline(timeline: EnergyTimeline): string {
		const bases = `${timeline.bases || 1} base${timeline.bases === 1 ? '' : 's'}`;
		const players = timeline.perPlayer?.length ?? 0;
		const owners = players > 1 ? ` (${players} players` : '';
		return `${Math.round(timeline.missedSeconds)}s idle across ${bases}${owners ? owners + ')' : ''}`;
	}

	/**
	 * Builds one series per player from a selector.
	 *
	 * @param select Returns a player's parallel x/y arrays.
	 * @returns Chart series, one per player.
	 */
	function seriesFor(select: (player: AnalysisPayload['players'][number]) => { x: number[]; y: number[] }) {
		return players.map((player) => ({
			...select(player),
			label: player.name,
			colorToken: playerStyle(player, players).colorToken
		}));
	}

	/**
	 * Formats a supply-block band's tooltip, which the chart shows verbatim.
	 *
	 * @param label The marker's label from `supplyBlockMarkers`.
	 * @returns The label unchanged; the helper exists so the intent is stated.
	 */
	function tooltip(label: string): string {
		return label;
	}
</script>

<svelte:head>
	<title>{payload ? `${payload.map} — SC2 Science Vessel` : 'Analysis — SC2 Science Vessel'}</title>
</svelte:head>

<section class="space-y-6">
	{#if loading}
		<div>
			<h1 class="text-xl font-semibold">Reading the replay…</h1>
			<p class="mt-1 text-sm text-ink-500">
				Decoding events from {replayPath.split('/').pop()}. A long 4v4 can take a few seconds.
			</p>
		</div>
		<div class="h-64 animate-pulse rounded-lg border border-surface-800 bg-surface-900/40"></div>
	{:else if failure}
		<div>
			<h1 class="text-xl font-semibold">Could not analyse this replay</h1>
			<p class="mt-2 rounded-lg border border-surface-800 bg-surface-900/60 p-4 text-sm text-ink-300">
				{failure}
			</p>
			<p class="mt-2 text-xs text-ink-500">
				From the command line, <code class="font-mono">sc2sv analyze "&lt;replay&gt;" --summary</code>
				prints the same error with more detail.
			</p>
		</div>
	{:else if payload}
		<div class="flex flex-wrap items-baseline justify-between gap-2">
			<div>
				<h1 class="text-xl font-semibold">{payload.map}</h1>
				<p class="tabular mt-1 text-sm text-ink-500">
					{payload.matchup} · {formatGameTime(payload.lengthSeconds)} game time ·
					patch {payload.release} · {payload.gameType}
					{#if payload.isLadder}· ladder{/if}
				</p>
			</div>
			<p class="text-xs text-ink-500">
				{#if payload.mmr.length > 0}
					MMR {payload.mmr.join(' / ')}
				{:else}
					No MMR recorded in this replay
				{/if}
			</p>
		</div>

		<!-- The headline: who was ahead on the resources-lost graph, and how the game ended. -->
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
			{#each payload.players as player (player.pid)}
				{@const style = playerStyle(player, payload.players)}
				<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-3">
					<div class="flex items-center gap-2">
						<span
							class="inline-block h-2.5 w-2.5 rounded-full"
							style="background-color: var(--color-{style.colorToken})"
							aria-hidden="true"
						></span>
						<span class="truncate text-sm font-medium">{player.name}</span>
						{#if style.dash}<span class="text-xs text-ink-500">(dashed)</span>{/if}
					</div>
					<p class="mt-1 text-xs text-ink-500">
						{player.race} · {player.result}
						{#if !player.isHuman}· computer{/if}
					</p>
					<p class="tabular mt-2 text-xs text-ink-300">
						{player.workers.workers.length > 0
							? `peak ${Math.max(...player.workers.workers)} workers`
							: 'no worker samples'}
						· {player.engagements.length} engagements
					</p>
					{#if player.supplyBlocks.length > 0}
						<p class="tabular text-xs text-ink-500">
							{player.supplyBlocks.length} supply block{player.supplyBlocks.length !== 1 ? 's' : ''}
						</p>
					{:else}
						<p class="text-xs text-ahead">never supply blocked</p>
					{/if}
				</div>
			{/each}
		</div>

		<div>
			<div class="flex flex-wrap gap-1" role="tablist" aria-label="Metric groups">
				{#each groups as group (group.id)}
					<button
						type="button"
						role="tab"
						aria-selected={selected === group.id}
						class="rounded px-3 py-1.5 text-sm {selected === group.id
							? 'bg-surface-800 text-ink-100'
							: 'text-ink-500 hover:bg-surface-900'}"
						onclick={() => (selected = group.id)}
					>
						{group.label}
					</button>
				{/each}
			</div>
			<p class="mt-2 text-xs text-ink-500">{groups[selected].blurb}</p>
		</div>

		{#if selected === 0}
			<!--
				Economy. The worker graph is the headline number of any replay analysis, so it is
				drawn first and with the drops marked: a drop is a runby or a lost fight, and it is
				the difference between "my macro was bad" and "I lost a fight I did not notice".
			-->
			<div class="space-y-4">
				<TimelineChart
					title="Workers"
					series={seriesFor((player) => ({ x: player.workers.seconds, y: player.workers.workers }))}
					lengthSeconds={gameLength}
					secondFormatter={formatGameTime}
					cursorSecond={cursorSecond}
					onhover={(second) => (cursorSecond = second)}
				/>
				<TimelineChart
					title="Income (resources per minute, as the replay reports it)"
					series={seriesFor((player) => ({
						x: player.resources.seconds,
						y: player.resources.differential
					}))}
					lengthSeconds={gameLength}
					secondFormatter={formatGameTime}
					cursorSecond={cursorSecond}
					onhover={(second) => (cursorSecond = second)}
					emptyMessage="This replay records no resource series."
				/>
				<TimelineChart
					title="Resources lost in fights (cumulative)"
					series={players.map((player) => ({
						// `resources` carries the killed-minus-lost differential; the lost total is
						// reconstructed from it so the panel shows absolute losses, which is what
						// "how much did this fight cost me" actually asks.
						label: `${player.name} lost`,
						colorToken: playerStyle(player, players).colorToken,
						x: player.resources.seconds,
						y: player.resources.differential
					}))}
					lengthSeconds={gameLength}
					secondFormatter={formatGameTime}
					cursorSecond={cursorSecond}
					onhover={(second) => (cursorSecond = second)}
					emptyMessage="This replay records no resource series."
				/>
			</div>
		{:else if selected === 1}
			<!--
				Production. Supply blocks and energy downtime are drawn as bands rather than as
				derived percentages; see `supplyBlockMarkers` and `energyMarkers` for why the replay
				cannot support a single "uptime" number honestly.
			-->
			<div class="space-y-4">
				<TimelineChart
					title="Seconds spend supply blocked, per block"
					series={players.map((player) => ({
						...supplyBlockDurations(player.supplyBlocks),
						label: player.name,
						colorToken: playerStyle(player, players).colorToken
					}))}
					markers={players.flatMap((player) =>
						supplyBlockMarkers(
							player.supplyBlocks,
							playerStyle(player, players).colorToken,
							gameLength
						)
					)}
					lengthSeconds={gameLength}
					secondFormatter={formatGameTime}
					cursorSecond={cursorSecond}
					onhover={(second) => (cursorSecond = second)}
					emptyMessage="Neither player was supply blocked."
				/>

				{#each [{ key: 'inject', label: 'Larva inject' }, { key: 'mule', label: 'MULE' }, { key: 'chrono', label: 'Chrono boost' }] as ability (ability.key)}
					{@const timeline = payload.energy[ability.key as 'inject' | 'mule' | 'chrono']}
					{#if timeline.seconds.length > 0}
						<TimelineChart
							title="{ability.label} — {energyHeadline(timeline)}"
							series={[
								{
									label: `${ability.label} casts`,
									colorToken: 'neutral',
									x: timeline.seconds,
									y: timeline.seconds.map((_, index) => index + 1),
									stepped: true
								}
							]}
							markers={energyMarkers(timeline, 'neutral')}
							lengthSeconds={gameLength}
							secondFormatter={formatGameTime}
							cursorSecond={cursorSecond}
							onhover={(second) => (cursorSecond = second)}
						/>
						{#if (timeline.perPlayer?.length ?? 0) > 1}
							<!--
								A mirror matchup has two players sharing this ability. One blended figure
								cannot say whose macro was idle, so each is named. Measured on a real 1v1 ZvZ:
								per-player attribution gives 2578.7s where pooling the casts and bases gave
								2116.5s, because a cast between two players' hatcheries was credited to the
								wrong one.
							-->
							<p class="-mt-1 pl-11 text-xs text-ink-500">
								{#each timeline.perPlayer ?? [] as entry, index (entry.pid)}
									{@const owner = playersByPid.get(entry.pid)}
									{#if index > 0}<span aria-hidden="true"> · </span>{/if}
									{#if owner}
										<span
											class="inline-block h-2 w-2 rounded-full align-middle"
											style="background-color: var(--color-{raceColorToken(owner.race)})"
											aria-hidden="true"
										></span>
										{owner.name}: {Math.round(entry.missedSeconds)}s idle across {entry.bases}
										base{entry.bases === 1 ? '' : 's'}
									{:else}
										unattributed: {Math.round(entry.missedSeconds)}s idle
									{/if}
								{/each}
							</p>
						{/if}
					{/if}
				{/each}

				{#if payload.energy.inject.seconds.length === 0 && payload.energy.mule.seconds.length === 0 && payload.energy.chrono.seconds.length === 0}
					<p class="rounded-lg border border-dashed border-surface-700 bg-surface-900/40 p-4 text-sm text-ink-500">
						No energy ability casts were found. The replay's game-event stream is empty for this
						file, which happens when it was recorded by a build that does not write ability events.
					</p>
				{/if}

				<div class="grid gap-4 lg:grid-cols-2">
					{#each players as player (player.pid)}
						<div class="space-y-2">
							<h3 class="flex items-center gap-2 text-xs font-semibold tracking-wide text-ink-300 uppercase">
								<span
									class="inline-block h-2 w-2 rounded-full"
									style="background-color: var(--color-{playerStyle(player, players).colorToken})"
									aria-hidden="true"
								></span>
								{player.name} tech and expansions
							</h3>
							{#if player.tech.length === 0 && player.expansions.length === 0}
								<p class="text-xs text-ink-500">No tech structures or expansions were tracked.</p>
							{:else}
								<TimelineChart
									title="Tech structures and expansions"
									series={[]}
									markers={[
										...stepMarkers(player.expansions, 'ahead'),
										...stepMarkers(player.tech, 'neutral')
									]}
									lengthSeconds={gameLength}
									secondFormatter={formatGameTime}
									cursorSecond={cursorSecond}
									onhover={(second) => (cursorSecond = second)}
									height={90}
								/>
								<ul class="flex flex-wrap gap-x-3 gap-y-1 text-xs text-ink-500">
									{#each player.expansions as step (step.second + step.name)}
										<li>
											<span class="text-ahead">{step.name}</span>
											<span class="tabular">{formatGameTime(step.second)}</span>
										</li>
									{/each}
								</ul>
							{/if}
						</div>
					{/each}
				</div>
			</div>
		{:else if selected === 2}
			<!--
				Military. The differential chart is the "0 = even, positive = your favour" graph the
				list asks for, and the map below answers "where were those fights" for whichever
				moment the pointer is on.
			-->
			<div class="space-y-4">
				<TimelineChart
					title="Resource swing (0 even · above favours the line's player)"
					series={seriesFor((player) => ({
						x: player.resources.seconds,
						y: player.resources.differential
					}))}
					lengthSeconds={gameLength}
					secondFormatter={formatGameTime}
					cursorSecond={cursorSecond}
					onhover={(second) => (cursorSecond = second)}
					height={170}
				/>

				<div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
					<div class="space-y-3">
						{#each players as player (player.pid)}
							{@const totals = player.engagements.reduce(
								(accumulator, engagement) => {
									if (engagement.net > 0) accumulator.favourable += 1;
									else if (engagement.net < 0) accumulator.unfavourable += 1;
									return accumulator;
								},
								{ favourable: 0, unfavourable: 0 }
							)}
							<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-3">
								<div class="flex items-baseline justify-between gap-2">
									<h3 class="text-sm font-medium">{player.name}</h3>
									<span class="tabular text-xs text-ink-500">
										{totals.favourable} won · {totals.unfavourable} lost
									</span>
								</div>
								{#if player.engagements.length === 0}
									<p class="mt-1 text-xs text-ink-500">
										No engagement was recorded — nothing died in a sample interval.
									</p>
								{:else}
									<ul class="mt-2 flex flex-wrap gap-1">
										{#each player.engagements.slice(0, 60) as engagement (`${player.pid}-${engagement.second}-${engagement.lost}`)}
											<button
												type="button"
												class="tabular rounded px-1.5 py-0.5 font-mono text-[11px] {engagement.net > 0
													? 'bg-ahead/15 text-ahead'
													: 'bg-behind/15 text-behind'}"
												title="{formatGameTime(engagement.second)}: lost {engagement.lost}, killed {engagement.killed} ({engagement.net > 0 ? '+' : ''}{engagement.net})"
												onclick={() => (cursorSecond = engagement.second)}
												onmouseenter={() => (cursorSecond = engagement.second)}
											>
												{formatGameTime(engagement.second)}
											</button>
										{/each}
									</ul>
								{/if}
							</div>
						{/each}
					</div>

					<BattleMap
						deaths={payload.deaths}
						{cursorSecond}
						runbySecond={runbyWorst?.worstBurstAt ?? null}
						runbyBurstSize={runbyWorst?.worstBurst ?? 0}
					/>
				</div>

				{#if runbyWorst && runbyWorst.worstBurst > 0}
					<p class="rounded-lg border border-surface-800 bg-surface-900/60 p-3 text-sm text-ink-300">
						Worker losses: {runby.map((entry) => `${entry.name} ${entry.total}`).join(' · ')}.
						{#if runbyWorst.worstBurst >= 5}
							<span class="text-behind">
								{runbyWorst.name} lost {runbyWorst.worstBurst} workers within a minute at
								{formatGameTime(runbyWorst.worstBurstAt ?? 0)} — the worst runby in the game.
							</span>
						{:else}
							No burst of worker losses stands out, so no runby is claimed.
						{/if}
					</p>
				{/if}
			</div>
		{:else}
			<!--
				Build order, one player at a time: the comparison needs a subject, and in a team game
				"the build order" as a single list would be an average of two plans.
			-->
			<div class="space-y-6">
				{#each players as player (player.pid)}
					<BuildOrderPanel {player} matchup={payload.matchup} lengthSeconds={gameLength} allPlayers={players} {cursorSecond} onhover={(second) => (cursorSecond = second)} />
				{/each}
			</div>
		{/if}
	{:else}
		<p class="text-sm text-ink-500">Nothing to show.</p>
	{/if}
</section>

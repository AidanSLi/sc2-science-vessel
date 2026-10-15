<script lang="ts">
	/**
	 * A schematic map of where units died.
	 *
	 * The replay records positions but no terrain, and rendering the real map needs the map file,
	 * which is not available offline. So this draws the *shape* of the fighting rather than a
	 * picture of Acropolis: coordinates are projected into a square that keeps the aspect ratio
	 * of the area actually used (see `projectToUnitSquare`), and the panel says so, because a
	 * terrain-less map that looked like a real one would invite the reader to read strategy into
	 * positions that have no context.
	 *
	 * What the schematic is genuinely good for is the question the feature list asks: *where* the
	 * fights happened and whether workers were dying somewhere far from the armies. Worker deaths
	 * are drawn as squares rather than circles precisely so a runby at an expansion is visible as
	 * a cluster of squares away from the main mass, and a burst of them is outlined.
	 */
	import { formatGameTime } from '#lib/clock';
	import { projectToUnitSquare, raceColorToken } from '#lib/analysis-view';
	import type { DeathLocation } from '#lib/analysis';

	interface Props {
		/** Every death with coordinates, for both players. */
		deaths: DeathLocation[];
		/** The second the caller is highlighting, or null. */
		cursorSecond?: number | null;
		/**
		 * Seconds either side of the cursor that count as "in this moment".
		 *
		 * Player stats are sampled every 10 game seconds, so the instant a battle is reported at
		 * is accurate to about that; a wider window keeps the map from flickering empty between
		 * ticks.
		 */
		windowSeconds?: number;
		/** The second the worst worker-loss burst started, used to outline its deaths. */
		runbySecond?: number | null;
		/** How many workers that burst killed; a burst this small is not outlined (see below). */
		runbyBurstSize?: number;
	}

	let {
		deaths,
		cursorSecond = null,
		windowSeconds = 12,
		runbySecond = null,
		runbyBurstSize = 0
	}: Props = $props();

	/** The projection, built once from every death so the picture does not shift as the cursor moves. */
	const projection = $derived(projectToUnitSquare(deaths));

	/** Deaths with coordinates, which is all but a handful in a normal game. */
	const located = $derived(deaths.filter((death) => death.x !== null && death.y !== null));

	/** Size of the drawing area in pixels; square, so the projection's aspect ratio is preserved. */
	const SIZE = 300;

	/**
	 * Places one death in the square.
	 *
	 * @param death A death with coordinates.
	 * @returns Pixel coordinates, after projecting and flipping y.
	 */
	function position(death: DeathLocation): [number, number] {
		const [x, y] = projection.project(death.x ?? 0, death.y ?? 0);
		// Replay y grows upward from the map's bottom-left corner while SVG y grows downward, so
		// flipping is what makes an attack at the top of a map draw at the top of the panel.
		return [x * SIZE, SIZE - y * SIZE];
	}

	/** How close to the cursor a death has to be to count as part of the highlighted moment. */
	const inWindow = $derived((death: DeathLocation) =>
		cursorSecond === null ? true : Math.abs(death.second - cursorSecond) <= windowSeconds
	);

	/** The most recent death at or before the cursor, for the caption. */
	const nearestAtCursor = $derived.by(() => {
		if (cursorSecond === null || located.length === 0) return null;
		let best: DeathLocation | null = null;
		for (const death of located) {
			if (death.second > cursorSecond) continue;
			if (best === null || death.second > best.second) best = death;
		}
		return best;
	});

	/** Workers lost in the highlighted window, which is the runby number. */
	const workersInWindow = $derived(
		located.filter((death) => death.isWorker && inWindow(death)).length
	);

	/** Units lost in the highlighted window. */
	const unitsInWindow = $derived(located.filter((death) => !death.isWorker && inWindow(death)).length);
</script>

<figure class="rounded-lg border border-surface-800 bg-surface-900/60 p-3">
	<figcaption class="mb-2 flex flex-wrap items-baseline justify-between gap-2">
		<h3 class="text-xs font-semibold tracking-wide text-ink-300 uppercase">Where units died</h3>
		<span class="tabular text-xs text-ink-500">
			{#if cursorSecond === null}
				{located.length} deaths plotted
			{:else}
				{formatGameTime(cursorSecond)} · {unitsInWindow} unit{#if unitsInWindow !== 1}s{/if},
				{workersInWindow} worker{#if workersInWindow !== 1}s{/if} lost
			{/if}
		</span>
	</figcaption>

	{#if located.length === 0}
		<div
			class="flex items-center justify-center rounded border border-dashed border-surface-700 text-xs text-ink-500"
			style="height: {SIZE}px"
		>
			No unit deaths were recorded in this game.
		</div>
	{:else}
		<svg
			viewBox="0 0 {SIZE} {SIZE}"
			class="mx-auto block h-auto w-full max-w-[{SIZE}px] touch-none select-none rounded border border-surface-800 bg-surface-950"
			role="img"
			aria-label="Schematic map of unit deaths; positions are relative, not drawn on terrain"
		>
			<!--
				A faint crosshair at the data's centre, so the schematic reads as a coordinate space
				rather than as a broken picture of a map. It carries no information and is not
				labelled as though it did.
			-->
			<line x1={SIZE / 2} y1="0" x2={SIZE / 2} y2={SIZE} stroke="var(--color-surface-850)" stroke-width="1" />
			<line x1="0" y1={SIZE / 2} x2={SIZE} y2={SIZE / 2} stroke="var(--color-surface-850)" stroke-width="1" />

			{#if runbyBurstSize >= 5}
				<!--
					A runby is detected as a burst — five or more workers inside a minute, see
					`workerLossSummary` — because the replay records where each worker died but nothing
					about intent, so position alone cannot identify one. The outline follows those
					deaths wherever they are. Below five the burst is ordinary attrition and outlining
					it would cry wolf every game.
				-->
				{#each located.filter((death) => death.isWorker && Math.abs(death.second - (runbySecond ?? 0)) <= 60) as death (death.second + death.unit + death.x)}
					{@const [rx, ry] = position(death)}
					<rect
						x={rx - 4}
						y={ry - 4}
						width="8"
						height="8"
						fill="none"
						stroke="var(--color-behind)"
						stroke-width="1"
					/>
				{/each}
			{/if}

			{#each located as death, index (index)}
				{@const [cx, cy] = position(death)}
				{@const active = inWindow(death)}
				{@const token = raceColorToken(death.ownerRace ?? '')}
				{#if death.isWorker}
					<rect
						x={cx - (active ? 2.6 : 1.8)}
						y={cy - (active ? 2.6 : 1.8)}
						width={active ? 5.2 : 3.6}
						height={active ? 5.2 : 3.6}
						fill="var(--color-{token})"
						opacity={active ? 0.95 : 0.35}
					>
						<title>{death.unit} lost at {formatGameTime(death.second)}</title>
					</rect>
				{:else}
					<circle
						cx={cx}
						cy={cy}
						r={active ? 3.2 : 1.8}
						fill="var(--color-{token})"
						opacity={active ? 0.95 : 0.3}
					>
						<title>{death.unit} lost at {formatGameTime(death.second)}</title>
					</circle>
				{/if}
			{/each}

			{#if nearestAtCursor !== null}
				{@const [cx, cy] = position(nearestAtCursor)}
				<circle
					cx={cx}
					cy={cy}
					r="7"
					fill="none"
					stroke="var(--color-ink-100)"
					stroke-width="1"
					opacity="0.7"
				/>
			{/if}

			<!--
				A caption inside the frame rather than below it, so it cannot be missed when the panel
				is read in isolation — the one thing a reader must not do is treat these positions as
				terrain.
			-->
			<text x="6" y={SIZE - 6} fill="var(--color-ink-500)" font-size="9">
				Relative positions; the replay records no terrain
			</text>
			<text x="6" y="12" fill="var(--color-ink-500)" font-size="9">Large marks: at the highlighted time</text>
		</svg>
	{/if}

	<ul class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-ink-300">
		<li class="flex items-center gap-1.5">
			<span class="inline-block h-2 w-2 rounded-full bg-ink-500"></span> fighting unit
		</li>
		<li class="flex items-center gap-1.5">
			<span class="inline-block h-2 w-2 bg-ink-500"></span> worker
		</li>
		<li class="text-ink-500">hover a chart to highlight one moment</li>
	</ul>
</figure>

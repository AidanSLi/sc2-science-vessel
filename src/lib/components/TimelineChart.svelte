<script lang="ts">
	/**
	 * A time-series panel on the shared game-clock axis.
	 *
	 * Every panel in the analysis view is the same component so that one hover position means
	 * the same moment in all of them, and so a chart is added by describing its series rather
	 * than by writing new SVG. The pointer position is reported upward rather than drawn as a
	 * local tooltip: the caller owns the "this second" state and passes it back in, which is
	 * what makes the cross-panel highlighting work.
	 *
	 * Deliberately no chart library (see `#lib/charts`). The panels are plain SVG so a panel
	 * can also carry non-series marks — supply-block bands, build-step ticks, worker-death
	 * stems — which are the annotations that actually make the charts readable and which a
	 * general-purpose library makes awkward.
	 */
	import {
		linearScale,
		linePath,
		scaleValue,
		secondToX,
		steppedPath,
		timeTicks
	} from '#lib/charts';

	/** One line to draw. */
	export interface Series {
		label: string;
		/** Colour token name from `app.css`, e.g. `race-zerg`. Never a raw colour. */
		colorToken: string;
		/** X values in game seconds, ascending. */
		x: number[];
		/** Y values, parallel to `x`. */
		y: number[];
		/** Draw as a stepped line. Correct for counts that hold between samples. */
		stepped?: boolean;
		/** Fill the area under the line. Used when a single series is the whole point. */
		filled?: boolean;
		/** Draw only the line, with no entry in the legend. For reference lines. */
		quiet?: boolean;
	}

	/** A vertical mark on one panel, in game seconds. */
	export interface Marker {
		/** Where the mark starts. */
		from: number;
		/** Where it ends; equal to `from` for an instant. */
		to: number;
		/** Tooltip text. */
		label: string;
		/** Colour token name from `app.css`. */
		colorToken: string;
		/** True when the mark's exact position within `[from, to]` is unknown. */
		uncertain?: boolean;
	}

	interface Props {
		/** Panel heading. */
		title: string;
		/** The series to plot. An empty list renders the panel's empty message. */
		series: Series[];
		/** The game's length in seconds, which fixes the x-axis. */
		lengthSeconds: number;
		/** Vertical marks drawn behind the lines. */
		markers?: Marker[];
		/** Shown instead of the chart when there is nothing to plot. */
		emptyMessage?: string;
		/** The second the caller is currently highlighting, or null. */
		cursorSecond?: number | null;
		/**
		 * Formats the pointer's second for the caller.
		 *
		 * The value is on the game clock, which is the only clock the app uses; callers that
		 * need `m:ss` pass `formatGameTime`. Kept as a prop so the figure does not have to
		 * decide how a time is spelled outside its own axis labels.
		 */
		secondFormatter?: (second: number) => string;
		/** Reports a pointer position, in game seconds, or null when the pointer leaves. */
		onhover?: (second: number | null) => void;
		/** Height of the drawing area in pixels. */
		height?: number;
	}

	let {
		title,
		series,
		lengthSeconds,
		markers = [],
		emptyMessage = 'Nothing to plot for this game.',
		cursorSecond = null,
		secondFormatter = (second: number) => `${Math.round(second)}s`,
		onhover,
		height = 140
	}: Props = $props();

	/**
	 * Margin around the plot, in pixels.
	 *
	 * The left margin holds the y-axis labels and the bottom one the time labels. Fixed rather
	 * than measured: the labels are short and known, and a layout-measuring pass would make the
	 * component's first paint empty, which reads as a broken chart.
	 */
	const PAD = { top: 8, right: 8, bottom: 18, left: 44 };

	/** Panel width, measured from the container so the chart fills whatever space it is given. */
	let width = $state(720);
	let svg: SVGSVGElement | null = $state(null);

	/**
	 * Keeps the drawing width in step with the element's actual size.
	 *
	 * An SVG with a viewBox would scale its strokes and text with the panel, which makes labels
	 * the wrong size at both extremes; drawing in real pixels and re-measuring avoids that.
	 */
	$effect(() => {
		if (!svg) return;
		const measure = () => {
			const measured = svg?.clientWidth ?? 0;
			if (measured > 0) width = measured;
		};
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(svg);
		return () => observer.disconnect();
	});

	const plotWidth = $derived(Math.max(40, width - PAD.left - PAD.right));
	const plotHeight = $derived(Math.max(24, height - PAD.top - PAD.bottom));

	/** Every y value across all series, for one shared y scale so the lines are comparable. */
	const allValues = $derived(series.flatMap((s) => s.y));
	const yScale = $derived(linearScale(allValues, PAD.top + plotHeight, PAD.top));

	/** The visible series, in draw order. Quiet series get no legend entry but are still drawn. */
	const legend = $derived(series.filter((s) => !s.quiet));

	/**
	 * Builds the SVG path for one series.
	 *
	 * @param s The series to draw.
	 * @returns The path's `d` attribute.
	 */
	function pathFor(s: Series): string {
		const xs = s.x.map((second) => secondToX(second, lengthSeconds, plotWidth) + PAD.left);
		const ys = s.y.map((value) => scaleValue(yScale, value));
		return s.stepped ? steppedPath(xs, ys) : linePath(xs, ys);
	}

	/**
	 * Builds the closed path for a filled series, which needs the axis as its baseline.
	 *
	 * @param s The series to fill.
	 * @returns The area's `d` attribute.
	 */
	function areaFor(s: Series): string {
		const line = pathFor(s);
		if (!line) return '';
		const xs = s.x.map((second) => secondToX(second, lengthSeconds, plotWidth) + PAD.left);
		const baseline = PAD.top + plotHeight;
		return `${line} L ${xs[xs.length - 1]} ${baseline} L ${xs[0]} ${baseline} Z`;
	}

	/**
	 * Converts a pointer event to a game second.
	 *
	 * @param event A pointer event on the plot area.
	 * @returns The second under the pointer, clamped to the game.
	 */
	function secondAt(event: PointerEvent): number {
		const rect = (event.currentTarget as SVGRectElement).getBoundingClientRect();
		const x = event.clientX - rect.left;
		const fraction = Math.max(0, Math.min(1, x / Math.max(1, plotWidth)));
		return fraction * lengthSeconds;
	}

	/** The highlight line's x coordinate, or null when nothing is highlighted. */
	const cursorX = $derived(
		cursorSecond === null ? null : secondToX(cursorSecond, lengthSeconds, plotWidth) + PAD.left
	);

	/**
	 * Shared x-axis tick positions.
	 *
	 * Derived from the game length rather than the panel width so a long game and a short one get
	 * round-numbered labels rather than however many fit. The label text itself is produced by
	 * `secondFormatter`, so this component does not decide how a time is spelled. The first tick
	 * (zero) doubles as the left gridline.
	 */
	const ticks = $derived(timeTicks(lengthSeconds));
</script>

<figure class="rounded-lg border border-surface-800 bg-surface-900/60 p-3">
	<figcaption class="mb-2 flex items-baseline justify-between gap-3">
		<h3 class="text-xs font-semibold tracking-wide text-ink-300 uppercase">{title}</h3>
		{#if secondFormatter && cursorSecond !== null}
			<span class="tabular text-xs text-ink-500">{secondFormatter(cursorSecond)}</span>
		{/if}
	</figcaption>

	{#if series.length === 0}
		<div
			class="flex items-center justify-center rounded border border-dashed border-surface-700 text-xs text-ink-500"
			style="height: {height}px"
		>
			{emptyMessage}
		</div>
	{:else}
		<svg
			bind:this={svg}
			class="block w-full touch-none select-none"
			height={height}
			role="img"
			aria-label={`${title} over game time`}
		>
			<!-- Gridlines first, so every line draws over them. Ticks carry their own line. -->
			{#each ticks as tick (tick)}
				<line
					x1={secondToX(tick, lengthSeconds, plotWidth) + PAD.left}
					x2={secondToX(tick, lengthSeconds, plotWidth) + PAD.left}
					y1={PAD.top}
					y2={PAD.top + plotHeight}
					stroke="var(--color-surface-800)"
					stroke-width="1"
				/>
			{/each}

			<!-- Zero line, drawn distinctly because "even" is the reference the reader compares to. -->
			{#if yScale.min < 0 && yScale.max > 0}
				<line
					x1={PAD.left}
					x2={PAD.left + plotWidth}
					y1={scaleValue(yScale, 0)}
					y2={scaleValue(yScale, 0)}
					stroke="var(--color-surface-700)"
					stroke-width="1"
					stroke-dasharray="3 3"
				/>
			{/if}

			<!-- Markers sit behind the data so a band never obscures the line it annotates. -->
			{#each markers as marker (marker.label + marker.from)}
				<rect
					x={secondToX(marker.from, lengthSeconds, plotWidth) + PAD.left}
					width={Math.max(
						1,
						secondToX(marker.to, lengthSeconds, plotWidth) -
							secondToX(marker.from, lengthSeconds, plotWidth)
					)}
					y={PAD.top}
					height={plotHeight}
					fill="var(--color-{marker.colorToken})"
					opacity={marker.uncertain ? 0.14 : 0.24}
				>
					<title>{marker.label}</title>
				</rect>
			{/each}

			{#each series as s (s.label)}
				{#if s.filled}
					<path d={areaFor(s)} fill="var(--color-{s.colorToken})" opacity="0.14" />
				{/if}
				<path
					d={pathFor(s)}
					fill="none"
					stroke="var(--color-{s.colorToken})"
					stroke-width={s.quiet ? 1 : 1.5}
					stroke-dasharray={s.quiet ? '4 3' : undefined}
					stroke-linejoin="round"
				/>
			{/each}

			<!-- Axis labels. Text uses the ink tokens so it stays legible on the panel. -->
			{#each ticks as tick (tick)}
				<text
					x={secondToX(tick, lengthSeconds, plotWidth) + PAD.left}
					y={PAD.top + plotHeight + 12}
					text-anchor={tick === 0 ? 'start' : 'middle'}
					fill="var(--color-ink-500)"
					font-size="10"
					class="tabular"
				>
					{secondFormatter(tick)}
				</text>
			{/each}

			{#each [yScale.max, yScale.min] as value, index (index)}
				<text
					x={PAD.left - 6}
					y={index === 0 ? PAD.top + 8 : PAD.top + plotHeight}
					text-anchor="end"
					fill="var(--color-ink-500)"
					font-size="10"
					class="tabular"
				>
					{Math.round(value)}
				</text>
			{/each}

			{#if cursorX !== null}
				<line
					x1={cursorX}
					x2={cursorX}
					y1={PAD.top}
					y2={PAD.top + plotHeight}
					stroke="var(--color-ink-300)"
					stroke-width="1"
					stroke-dasharray="2 2"
				/>
			{/if}

			<!--
				The capture layer comes last so it is on top and receives every pointer event.
				Hovering is a convenience over the axis labels, not a way to reach the data, so the layer
				is marked presentational: the numbers it reveals are also printed beside each panel.
			-->
			<rect
				x={PAD.left}
				y={PAD.top}
				width={plotWidth}
				height={plotHeight}
				fill="transparent"
				role="presentation"
				aria-hidden="true"
				onpointermove={(event) => onhover?.(secondAt(event))}
				onpointerleave={() => onhover?.(null)}
			/>
		</svg>
	{/if}

	{#if legend.length > 0}
		<ul class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs">
			{#each legend as s (s.label)}
				<li class="flex items-center gap-1.5 text-ink-300">
					<span
						class="inline-block h-2 w-2 shrink-0 rounded-full"
						style="background: var(--color-{s.colorToken})"
					></span>
					{s.label}
				</li>
			{/each}
		</ul>
	{/if}
</figure>

/**
 * Geometry for the analysis charts, as pure functions.
 *
 * There is no charting library here on purpose. Every chart this app draws is a time series
 * on one shared x-axis — the game clock — with the map underneath, and the whole point of the
 * analysis view is that hovering one point in time highlights *all* of them at once. That
 * cross-panel linking is simpler to do directly against an SVG than to negotiate with a
 * library's internal state, and it keeps the bundle free of a dependency that would need to
 * track Svelte 5's reactivity to be useful.
 *
 * Everything here is pure and returns numbers or path strings, so the maths is testable without
 * a DOM. Rendering decisions (colours, labels, tooltips) belong to the components.
 */

/** A numeric range along one axis, plus the function that maps a value into it. */
export interface Scale {
	/** The data range this scale was built from. */
	readonly min: number;
	/** Upper bound of the data range. */
	readonly max: number;
	/** Output range, in pixels. */
	readonly outMin: number;
	/** Upper bound of the output range. */
	readonly outMax: number;
}

/**
 * Builds a linear scale, widening a degenerate range instead of dividing by zero.
 *
 * A game where a value never changes (a player who never lost a worker, so `differential` is
 * flat at 0) gives `min === max`, and every point would map to `NaN`. Widening to `max ± 1`
 * keeps the line at the centre of its panel, which is the honest rendering of "this never
 * moved" — the alternative is either a blank chart or a line pinned to an edge implying a
 * change that did not happen.
 *
 * @param values The data to fit, which may be empty.
 * @param outMin Output coordinate for the data minimum.
 * @param outMax Output coordinate for the data maximum, which may be smaller than `outMin`
 *     for a y-axis that grows downward as SVG's does.
 * @returns A scale that never divides by zero.
 */
export function linearScale(values: readonly number[], outMin: number, outMax: number): Scale {
	let min = Number.POSITIVE_INFINITY;
	let max = Number.NEGATIVE_INFINITY;
	for (const value of values) {
		if (!Number.isFinite(value)) continue;
		if (value < min) min = value;
		if (value > max) max = value;
	}
	if (!Number.isFinite(min) || !Number.isFinite(max)) {
		min = 0;
		max = 1;
	}
	if (min === max) {
		min -= 1;
		max += 1;
	}
	return { min, max, outMin, outMax };
}

/**
 * Maps a value through a scale.
 *
 * @param scale A scale from {@link linearScale}.
 * @param value The value to place.
 * @returns The output coordinate.
 */
export function scaleValue(scale: Scale, value: number): number {
	const fraction = (value - scale.min) / (scale.max - scale.min);
	return scale.outMin + fraction * (scale.outMax - scale.outMin);
}

/**
 * Maps a game-clock second onto the shared x-axis.
 *
 * Every panel in the analysis view uses the same x scale so a vertical gridline means the same
 * moment everywhere, which is what makes hovering one panel readable against the others.
 *
 * @param lengthSeconds The game's length, which fixes the right edge.
 * @param width The drawable width in pixels.
 * @returns A pair of x coordinates, equal when the game has no duration.
 */
export function timeAxis(lengthSeconds: number, width: number): { x0: number; x1: number } {
	if (!Number.isFinite(lengthSeconds) || lengthSeconds <= 0) return { x0: 0, x1: width };
	return { x0: 0, x1: width };
}

/**
 * Converts a game second to an x coordinate on the shared axis.
 *
 * @param second The game-clock second.
 * @param lengthSeconds The game's length.
 * @param width The drawable width in pixels.
 * @returns The x coordinate, clamped to the panel.
 */
export function secondToX(second: number, lengthSeconds: number, width: number): number {
	if (!Number.isFinite(lengthSeconds) || lengthSeconds <= 0) return 0;
	const fraction = second / lengthSeconds;
	return Math.max(0, Math.min(1, fraction)) * width;
}

/**
 * Builds an SVG path that steps between values, as a build-order timeline should.
 *
 * A worker count changes at a sample and then holds until the next one, so a diagonal line
 * between samples implies a gradual change that did not happen. A stepped path is the accurate
 * rendering; the caller decides whether that matters for a given series.
 *
 * @param xs X coordinates, in order.
 * @param ys Y coordinates, parallel to `xs`.
 * @returns An SVG path `d` string, or an empty string when there is nothing to draw.
 */
export function steppedPath(xs: readonly number[], ys: readonly number[]): string {
	if (xs.length === 0 || xs.length !== ys.length) return '';
	let path = `M ${round(xs[0])} ${round(ys[0])}`;
	for (let i = 1; i < xs.length; i += 1) {
		// Horizontal to the new x at the old y, then vertical: the value held until this sample.
		path += ` H ${round(xs[i])} V ${round(ys[i])}`;
	}
	return path;
}

/**
 * Builds an SVG path through points, joined by straight lines.
 *
 * @param xs X coordinates, in order.
 * @param ys Y coordinates, parallel to `xs`.
 * @returns An SVG path `d` string, or an empty string when there is nothing to draw.
 */
export function linePath(xs: readonly number[], ys: readonly number[]): string {
	if (xs.length === 0 || xs.length !== ys.length) return '';
	let path = `M ${round(xs[0])} ${round(ys[0])}`;
	for (let i = 1; i < xs.length; i += 1) {
		path += ` L ${round(xs[i])} ${round(ys[i])}`;
	}
	return path;
}

/**
 * Finds the index of the sample nearest a target x coordinate.
 *
 * This is how a hover anywhere in a panel resolves to one moment in time. It compares in x
 * space rather than in seconds so it stays correct when the panel is resized or when a series
 * is sparse, and it works on a descending axis too because it only compares distances.
 *
 * @param xs Sorted x coordinates.
 * @param target The x coordinate the pointer is at.
 * @returns The nearest index, or -1 when there are no samples.
 */
export function nearestIndex(xs: readonly number[], target: number): number {
	if (xs.length === 0) return -1;
	let best = 0;
	let bestDistance = Number.POSITIVE_INFINITY;
	for (let i = 0; i < xs.length; i += 1) {
		const distance = Math.abs(xs[i] - target);
		if (distance < bestDistance) {
			bestDistance = distance;
			best = i;
		}
	}
	return best;
}

/**
 * Chooses readable tick positions for a time axis.
 *
 * Ticks land on round numbers so the gridlines can be labelled `5:00`, `10:00` and so on. The
 * step is chosen from a fixed ladder rather than computed, because a computed step produces
 * labels like `3:20`, `6:40`, which are harder to read against than round minutes.
 *
 * @param lengthSeconds The game's length.
 * @param targetCount How many ticks are wanted, approximately.
 * @returns Tick positions in game seconds, always including 0.
 */
export function timeTicks(lengthSeconds: number, targetCount = 6): number[] {
	if (!Number.isFinite(lengthSeconds) || lengthSeconds <= 0) return [0];
	const ladder = [15, 30, 60, 120, 300, 600, 900, 1200, 1800];
	const ideal = lengthSeconds / Math.max(1, targetCount);
	let step = ladder[ladder.length - 1];
	for (const candidate of ladder) {
		if (candidate >= ideal) {
			step = candidate;
			break;
		}
	}
	const ticks: number[] = [];
	for (let second = 0; second <= lengthSeconds; second += step) ticks.push(second);
	return ticks;
}

/**
 * Rounds a coordinate to one decimal place, keeping SVG paths short.
 *
 * Paths are rebuilt on every pointer move, so the coordinate strings dominate the attribute.
 * At a panel width under 1000px a tenth of a pixel is well below what a display can resolve.
 *
 * @param value A coordinate.
 * @returns The rounded value.
 */
function round(value: number): number {
	return Math.round(value * 10) / 10;
}

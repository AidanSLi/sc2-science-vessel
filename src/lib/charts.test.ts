/**
 * Tests for the chart geometry.
 *
 * These are worth testing because a scale bug produces a chart that looks plausible — a line
 * drawn in the wrong place is not obviously wrong at a glance, and the feature list's whole
 * point is reading those lines. Each case below is a situation that has actually come up in
 * the analysis payload: a flat series, an empty series, a one-point series, and a series whose
 * values straddle zero with the axis growing downward as SVG's does.
 */

import { describe, expect, it } from 'vitest';
import {
	linePath,
	linearScale,
	nearestIndex,
	scaleValue,
	secondToX,
	steppedPath,
	timeTicks
} from './charts';

describe('linearScale', () => {
	it('maps the data range onto the output range', () => {
		const scale = linearScale([0, 10], 100, 0);
		expect(scaleValue(scale, 0)).toBe(100);
		expect(scaleValue(scale, 10)).toBe(0);
		expect(scaleValue(scale, 5)).toBe(50);
	});

	it('widens a flat series instead of dividing by zero', () => {
		// A player who never lost a worker has a resource differential of exactly 0 throughout.
		const scale = linearScale([0, 0, 0], 100, 0);
		const mapped = scaleValue(scale, 0);
		expect(Number.isFinite(mapped)).toBe(true);
		expect(mapped).toBe(50);
	});

	it('handles an empty series without producing NaN', () => {
		const scale = linearScale([], 100, 0);
		expect(Number.isFinite(scaleValue(scale, 0))).toBe(true);
	});

	it('ignores non-finite values rather than poisoning the range', () => {
		// Only the 4 survives, so the range is widened around it; what matters is that the NaN
		// and the infinity neither extend the range nor leave it unusable.
		const scale = linearScale([Number.NaN, 4, Number.POSITIVE_INFINITY], 0, 100);
		expect(scale.min).toBeLessThan(4);
		expect(scale.max).toBeGreaterThan(4);
		expect(Number.isFinite(scaleValue(scale, 4))).toBe(true);
	});

	it('handles a single point by centring it', () => {
		const scale = linearScale([7], 0, 100);
		expect(scaleValue(scale, 7)).toBe(50);
	});
});

describe('secondToX', () => {
	it('places a second proportionally along the axis', () => {
		expect(secondToX(0, 100, 400)).toBe(0);
		expect(secondToX(50, 100, 400)).toBe(200);
		expect(secondToX(100, 100, 400)).toBe(400);
	});

	it('clamps a second past the end of the game rather than drawing off-panel', () => {
		// A build step at 12:01 in a 12:00 game is within the +/-10s tolerance, not an error.
		expect(secondToX(105, 100, 400)).toBe(400);
	});

	it('returns 0 for a game with no duration instead of dividing by zero', () => {
		expect(secondToX(10, 0, 400)).toBe(0);
		expect(secondToX(10, Number.NaN, 400)).toBe(0);
	});
});

describe('steppedPath', () => {
	it('moves horizontally then vertically, never diagonally', () => {
		// Worker counts hold between samples, so a diagonal would imply a gradual change.
		const path = steppedPath([0, 10], [0, 100]);
		expect(path).toBe('M 0 0 H 10 V 100');
		expect(path).not.toContain('L');
	});

	it('returns an empty string when the arrays disagree or are empty', () => {
		expect(steppedPath([], [])).toBe('');
		expect(steppedPath([1, 2], [1])).toBe('');
	});
});

describe('linePath', () => {
	it('joins points with straight segments', () => {
		expect(linePath([0, 10, 20], [0, 50, 100])).toBe('M 0 0 L 10 50 L 20 100');
	});

	it('returns an empty string for empty input', () => {
		expect(linePath([], [])).toBe('');
	});
});

describe('nearestIndex', () => {
	it('finds the sample nearest the pointer', () => {
		const xs = [0, 10, 20, 30];
		expect(nearestIndex(xs, 11)).toBe(1);
		expect(nearestIndex(xs, 16)).toBe(2);
		expect(nearestIndex(xs, -5)).toBe(0);
		expect(nearestIndex(xs, 999)).toBe(3);
	});

	it('returns -1 for an empty series', () => {
		expect(nearestIndex([], 5)).toBe(-1);
	});
});

describe('timeTicks', () => {
	it('always starts at zero so the left edge is labelled', () => {
		expect(timeTicks(194.8)[0]).toBe(0);
	});

	it('uses round steps so labels read as whole minutes', () => {
		const ticks = timeTicks(1200, 6);
		for (const tick of ticks) {
			const remainder = tick % 60;
			expect(remainder === 0 || tick % 30 === 0).toBe(true);
		}
	});

	it('never returns a tick past the end of the game', () => {
		for (const length of [45, 194.8, 400, 1200, 2922]) {
			for (const tick of timeTicks(length)) {
				expect(tick).toBeLessThanOrEqual(length);
			}
		}
	});

	it('degrades to a single tick for a degenerate game', () => {
		expect(timeTicks(0)).toEqual([0]);
		expect(timeTicks(Number.NaN)).toEqual([0]);
	});

	it('does not loop forever on a very short game', () => {
		const ticks = timeTicks(2, 6);
		expect(ticks.length).toBeLessThan(10);
	});
});

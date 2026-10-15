/**
 * Tests for the bundled build library.
 *
 * These run against the real `builds.json`, not a stub: the whole risk with this module is that
 * it mis-reads what SpawningTool actually wrote, and a hand-written fixture would encode the same
 * misunderstanding as the code. Two of the assertions below (merged rows, unsorted steps) exist
 * because real snapshot data violated the obvious assumption.
 */
import { describe, expect, it } from 'vitest';
import {
	buildsForMatchup,
	bundledBuilds,
	bundledSnapshotDate,
	bundledSnapshotSource,
	isKnownMatchup,
	referenceSteps,
	splitRepeats,
	toBuildOrder
} from './build-library';

describe('splitRepeats', () => {
	it('splits the repeat suffix SpawningTool writes into the name', () => {
		expect(splitRepeats('Drone x2')).toEqual({ name: 'Drone', repeats: 2 });
		expect(splitRepeats('Marine x10')).toEqual({ name: 'Marine', repeats: 10 });
	});

	it('leaves an unmerged name alone', () => {
		expect(splitRepeats('Pylon')).toEqual({ name: 'Pylon', repeats: 1 });
		expect(splitRepeats('Supply Depot')).toEqual({ name: 'Supply Depot', repeats: 1 });
	});

	it('does not treat a trailing number without the x prefix as a repeat', () => {
		// `2` alone is not a repeat count; treating it as one would rename a unit called `2`.
		expect(splitRepeats('Medivac 2')).toEqual({ name: 'Medivac 2', repeats: 1 });
	});

	it('ignores a zero or negative count rather than deleting the step', () => {
		expect(splitRepeats('Drone x0')).toEqual({ name: 'Drone', repeats: 1 });
	});
});

describe('toBuildOrder', () => {
	it('expands a merged row into one step per repetition', () => {
		const build = toBuildOrder({
			id: 1,
			name: 'Test',
			matchup: 'ZvT',
			steps: [
				{ supply: 14, second: 100, name: 'Zergling x4', category: 'Unit', note: '' },
				{ supply: 16, second: 120, name: 'Queen', category: 'Unit', note: '' }
			]
		});
		expect(build.steps.map((step) => step.name)).toEqual([
			'Zergling',
			'Zergling',
			'Zergling',
			'Zergling',
			'Queen'
		]);
		// All four copies keep the merged row's time, because that is the only time SpawningTool
		// recorded; they were simultaneous in the original build order.
		expect(build.steps.slice(0, 4).map((step) => step.timeInGameSeconds)).toEqual([100, 100, 100, 100]);
	});

	it('reads the chrono boost annotation off the name', () => {
		const build = toBuildOrder({
			id: 2,
			name: 'Test',
			matchup: 'PvZ',
			steps: [{ supply: 20, second: 150, name: 'Probe (Chrono Boost)', category: 'Worker', note: '' }]
		});
		expect(build.steps).toHaveLength(1);
		expect(build.steps[0].name).toBe('Probe');
		expect(build.steps[0].isChronoboosted).toBe(true);
		expect(build.steps[0].isWorker).toBe(true);
	});

	it('handles a row that is both merged and chronoboosted', () => {
		// SpawningTool nests the repeat count inside the chrono annotation — the one such row in the
		// bundled snapshot reads `Probe x2 (Chrono Boost)`. Both facts therefore have to come off,
		// and in the order the data dictates: the repeat pattern is anchored to the end of the
		// string, so the chrono suffix has to go first or the count is never matched. Getting this
		// backwards leaves `Probe x2` glued together, which matches nothing in a replay.
		const build = toBuildOrder({
			id: 7,
			name: 'Test',
			matchup: 'PvZ',
			steps: [{ supply: 20, second: 150, name: 'Probe x2 (Chrono Boost)', category: 'Worker', note: '' }]
		});
		expect(build.steps.map((step) => step.name)).toEqual(['Probe', 'Probe']);
		expect(build.steps.every((step) => step.isChronoboosted)).toBe(true);
	});

	it('carries the note, which is the whole instruction for an `Action` step', () => {
		const build = toBuildOrder({
			id: 8,
			name: 'Test',
			matchup: 'PvT',
			steps: [{ supply: 38, second: 222, name: 'Action', category: 'Unit', note: 'Hallucinate Phoenix, send to Terran main' }]
		});
		expect(build.steps[0].name).toBe('Action');
		expect(build.steps[0].note).toBe('Hallucinate Phoenix, send to Terran main');
	});

	it('normalises an empty note to null so the UI can test one thing', () => {
		const build = toBuildOrder({
			id: 9,
			name: 'Test',
			matchup: 'PvZ',
			steps: [{ supply: 12, second: 30, name: 'Pylon', category: 'Building', note: '   ' }]
		});
		expect(build.steps[0].note).toBeNull();
	});

	it('sorts the steps onto the clock', () => {
		// A curated build in the bundled snapshot really does list a later step first; order is
		// load-bearing for both `advance` in the overlay and `compareToBuild`, so it is fixed here.
		const build = toBuildOrder({
			id: 3,
			name: 'Test',
			matchup: 'TvZ',
			steps: [
				{ supply: 30, second: 278, name: 'Raven', category: 'Unit', note: '' },
				{ supply: 16, second: 260, name: 'Refinery', category: 'Building', note: '' }
			]
		});
		expect(build.steps.map((step) => step.name)).toEqual(['Refinery', 'Raven']);
	});

	it('tags the source as bundled and names SpawningTool as the author', () => {
		const build = toBuildOrder({ id: 4, name: 'Test', matchup: 'ZvZ', steps: [] });
		expect(build.source).toBe('bundled');
		expect(build.author).toBe('SpawningTool');
		expect(build.id).toBe('spawningtool-4');
	});
});

describe('the bundled snapshot', () => {
	it('parsed at all', () => {
		expect(bundledBuilds.length).toBeGreaterThan(0);
		expect(bundledSnapshotSource).toContain('spawningtool');
		expect(Number.isNaN(Date.parse(bundledSnapshotDate))).toBe(false);
	});

	it('gives every build a recognised matchup', () => {
		for (const build of bundledBuilds) {
			expect(isKnownMatchup(build.matchup)).toBe(true);
		}
	});

	it('gives every build a non-empty step list in time order', () => {
		for (const build of bundledBuilds) {
			expect(build.steps.length).toBeGreaterThan(0);
			for (let i = 1; i < build.steps.length; i += 1) {
				expect(build.steps[i].timeInGameSeconds).toBeGreaterThanOrEqual(build.steps[i - 1].timeInGameSeconds);
			}
		}
	});

	it('keeps every step inside the game clock the app uses', () => {
		// Step times are already game seconds from the scraper. A step past ~20 minutes in a
		// *guide* would mean the 22.4 fps scale leaked back in; the longest bundled build is a
		// long macro game, so a generous ceiling still catches that mistake.
		for (const build of bundledBuilds) {
			const last = build.steps[build.steps.length - 1].timeInGameSeconds;
			expect(last).toBeLessThan(30 * 60);
		}
	});

	it('expanded at least one merged row, which is why the splitting exists', () => {
		const merged = bundledBuilds.some((build) =>
			build.steps.some((step, index) => index > 0 && step.name === build.steps[index - 1].name)
		);
		expect(merged).toBe(true);
	});
});

describe('isKnownMatchup', () => {
	it('accepts the exact two-race matchups', () => {
		for (const matchup of ['TvT', 'TvP', 'TvZ', 'PvP', 'PvZ', 'ZvZ', 'ZvT', 'ZvP']) {
			expect(isKnownMatchup(matchup)).toBe(true);
		}
	});

	it('accepts the X wildcard, which a game can never be', () => {
		expect(isKnownMatchup('ZvX')).toBe(true);
	});

	it('rejects nonsense', () => {
		expect(isKnownMatchup('')).toBe(false);
		expect(isKnownMatchup('Random')).toBe(false);
		expect(isKnownMatchup('ZvQ')).toBe(false);
	});
});

describe('buildsForMatchup', () => {
	it('returns exact matchups before the wildcard', () => {
		const candidates = buildsForMatchup('ZvT');
		const wildcardFrom = candidates.findIndex((build) => build.matchup === 'ZvX');
		const exactTo = candidates.map((build) => build.matchup).lastIndexOf('ZvT');
		if (wildcardFrom !== -1 && exactTo !== -1) {
			expect(exactTo).toBeLessThan(wildcardFrom);
		}
	});

	it('never returns another race pair', () => {
		for (const build of buildsForMatchup('ZvT')) {
			expect(['ZvT', 'ZvX']).toContain(build.matchup);
		}
	});

	it('returns nothing for a matchup with no builds rather than everything', () => {
		expect(buildsForMatchup('QvQ')).toEqual([]);
	});
});

describe('referenceSteps', () => {
	it('renames the time field for the matcher', () => {
		const build = toBuildOrder({
			id: 5,
			name: 'Test',
			matchup: 'ZvZ',
			steps: [{ supply: 16, second: 42, name: 'Spawning Pool', category: 'Building', note: '' }]
		});
		expect(referenceSteps(build)).toEqual([{ name: 'Spawning Pool', second: 42, supply: 16, note: null }]);
	});

	it('keeps a missing supply as null rather than zero', () => {
		// Supply 0 is a real value (a drone at game start has supply null in SpawningTool's data
		// for abilities, and 0 would read as "at zero supply"), so an absent supply must survive
		// the translation to the matcher's shape as null. `referenceSteps` takes a build order, so
		// the step is built by hand here: the snapshot itself happens to have no nulls today, and
		// the point of the test is the contract, not the current data.
		const steps = referenceSteps({
			id: 'test',
			title: 'Test',
			matchup: 'ZvZ',
			source: 'bundled',
			steps: [
				{ supply: 0, timeInGameSeconds: 1, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
				{ supply: null as unknown as number, timeInGameSeconds: 2, name: 'Action', isWorker: false, isChronoboosted: false, note: 'scout' }
			]
		});
		expect(steps[0].supply).toBe(0);
		expect(steps[1].supply).toBeNull();
	});
});

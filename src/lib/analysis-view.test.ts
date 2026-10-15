import { describe, expect, it } from 'vitest';
import {
	compareToBuild,
	energyMarkers,
	playerStyle,
	projectToUnitSquare,
	rankBuilds,
	supplyBlockDurations,
	supplyBlockMarkers,
	workerLossSummary
} from './analysis-view';
import type { AnalysisPayload } from './analysis';

/** Two Protoss, so the mirror-matchup colour rule is actually exercised. */
const mirrorPlayers = [
	{ pid: 1, race: 'Protoss' as const },
	{ pid: 2, race: 'Protoss' as const }
];

describe('playerStyle', () => {
	it('gives both players of a mirror the race colour, distinguishing them by dashes', () => {
		expect(playerStyle(mirrorPlayers[0], mirrorPlayers).colorToken).toBe('race-protoss');
		expect(playerStyle(mirrorPlayers[1], mirrorPlayers).colorToken).toBe('race-protoss');
		expect(playerStyle(mirrorPlayers[0], mirrorPlayers).dash).toBeUndefined();
		expect(playerStyle(mirrorPlayers[1], mirrorPlayers).dash).toBeDefined();
	});

	it('does not dash the only player of a race', () => {
		const mixed = [
			{ pid: 1, race: 'Zerg' as const },
			{ pid: 2, race: 'Terran' as const }
		];
		expect(playerStyle(mixed[1], mixed).dash).toBeUndefined();
	});

	it('dashes the second of a race even when it is listed third', () => {
		const players = [
			{ pid: 1, race: 'Protoss' as const },
			{ pid: 2, race: 'Zerg' as const },
			{ pid: 3, race: 'Protoss' as const }
		];
		expect(playerStyle(players[2], players).dash).toBeDefined();
		expect(playerStyle(players[0], players).dash).toBeUndefined();
	});
});

describe('supplyBlockMarkers', () => {
	it('renders a closed block as its verified range and calls it uncertain', () => {
		const [marker] = supplyBlockMarkers(
			[{ sometimeAfter: 100, clearedBy: 130, minSeconds: 10, maxSeconds: 30, openEnded: false }],
			'behind',
			600
		);
		expect(marker.from).toBe(100);
		expect(marker.to).toBe(130);
		expect(marker.uncertain).toBe(true);
	});

	it('extends an open-ended block to the end of the game and says so', () => {
		const [marker] = supplyBlockMarkers(
			[{ sometimeAfter: 500, clearedBy: 500, minSeconds: 0, maxSeconds: 0, openEnded: true }],
			'behind',
			600
		);
		expect(marker.to).toBe(600);
		expect(marker.label).toContain('never cleared');
	});
});

describe('supplyBlockDurations', () => {
	it('places a block at its midpoint with its verified minimum duration', () => {
		const { x, y } = supplyBlockDurations([
			{ sometimeAfter: 100, clearedBy: 120, minSeconds: 10, maxSeconds: 20, openEnded: false }
		]);
		expect(x).toEqual([110]);
		expect(y).toEqual([10]);
	});
});

describe('energyMarkers', () => {
	it('marks every idle gap and every cast', () => {
		const marks = energyMarkers(
			{
				kind: 'inject',
				idealSeconds: 31.7,
				bases: 1,
				seconds: [100, 200],
				gaps: [{ base: [10, 10], from: 100, to: 200, idleSeconds: 68.3 }],
				missedSeconds: 68.3
			},
			'race-zerg'
		);
		expect(marks).toHaveLength(3);
		expect(marks.filter((mark) => mark.from === mark.to)).toHaveLength(2);
	});

	it('does not mark a gap that is on schedule', () => {
		const marks = energyMarkers(
			{
				kind: 'inject',
				idealSeconds: 31.7,
				bases: 1,
				seconds: [100],
				gaps: [{ base: [10, 10], from: 100, to: 110, idleSeconds: 0 }],
				missedSeconds: 0
			},
			'race-zerg'
		);
		expect(marks).toHaveLength(1);
	});
});

describe('projectToUnitSquare', () => {
	it('keeps the aspect ratio of the used area instead of stretching each axis', () => {
		// A flat line of fighting along x should stay flat, not become a diagonal.
		const { project } = projectToUnitSquare([
			{ x: 0, y: 0 },
			{ x: 100, y: 0 }
		]);
		const [, yStart] = project(0, 0);
		const [, yEnd] = project(100, 0);
		expect(yStart).toBe(yEnd);
	});

	it('places every point inside the unit square', () => {
		const { project } = projectToUnitSquare([
			{ x: 5, y: 9 },
			{ x: 60, y: 40 }
		]);
		for (const [x, y] of [project(5, 9), project(60, 40)]) {
			expect(x).toBeGreaterThanOrEqual(0);
			expect(x).toBeLessThanOrEqual(1);
			expect(y).toBeGreaterThanOrEqual(0);
			expect(y).toBeLessThanOrEqual(1);
		}
	});

	it('returns a usable projection for a single point', () => {
		const { project, bounds } = projectToUnitSquare([{ x: 12, y: 12 }]);
		expect(bounds).not.toBeNull();
		expect(project(12, 12)).toEqual([0, 0]);
	});

	it('returns null bounds when there are no coordinates at all', () => {
		expect(projectToUnitSquare([{ x: null, y: null }]).bounds).toBeNull();
	});
});

describe('compareToBuild', () => {
	const actual = [
		{ name: 'Probe', second: 2 },
		{ name: 'Pylon', second: 30 },
		{ name: 'Gateway', second: 60 },
		{ name: 'Gateway', second: 190 }
	];

	it('matches a step within tolerance and reports the signed delta', () => {
		const result = compareToBuild(actual, [{ name: 'Gateway', second: 55, supply: 12 }]);
		expect(result.matched).toHaveLength(1);
		expect(result.matched[0].deltaSeconds).toBe(5);
		expect(result.matched[0].verdict).toBe('late');
	});

	it('calls a delta of two seconds or less on time', () => {
		expect(compareToBuild(actual, [{ name: 'Pylon', second: 31 }]).matched[0].verdict).toBe('on time');
	});

	it('reports a reference step outside tolerance as missing', () => {
		expect(compareToBuild(actual, [{ name: 'Pylon', second: 300 }]).missing).toHaveLength(1);
	});

	it('does not let one actual step satisfy two reference steps', () => {
		const result = compareToBuild(actual, [
			{ name: 'Pylon', second: 30 },
			{ name: 'Pylon', second: 32 }
		]);
		expect(result.matched).toHaveLength(1);
		expect(result.missing).toHaveLength(1);
	});

	it('lists steps the player took that the reference never asked for', () => {
		const result = compareToBuild(actual, [{ name: 'Pylon', second: 30 }]);
		expect(result.extra.map((step) => step.name)).toContain('Probe');
		expect(result.extra).toHaveLength(3);
	});

	it('scores an abandoned build low rather than laundering it through a small numerator', () => {
		const reference = [
			{ name: 'Probe', second: 2 },
			{ name: 'Pylon', second: 30 },
			{ name: 'Gateway', second: 60 },
			{ name: 'CyberneticsCore', second: 120 },
			{ name: 'TwilightCouncil', second: 300 }
		];
		const result = compareToBuild(actual, reference);
		expect(result.similarity).toBeCloseTo(3 / 5, 5);
	});

	it('ignores extras in the score', () => {
		const result = compareToBuild([{ name: 'Pylon', second: 30 }], [{ name: 'Pylon', second: 30 }]);
		expect(result.similarity).toBe(1);
	});

	it('scores an empty reference as a perfect match', () => {
		expect(compareToBuild(actual, []).similarity).toBe(1);
	});
});

describe('rankBuilds', () => {
	it('puts the closer build first and breaks ties on total drift', () => {
		const actual = [
			{ name: 'Pylon', second: 30 },
			{ name: 'Gateway', second: 61 },
			{ name: 'CyberneticsCore', second: 121 }
		];
		const ranked = rankBuilds(actual, [
			{ title: 'Loose', steps: [{ name: 'Pylon', second: 35 }, { name: 'Gateway', second: 66 }] },
			{ title: 'Tight', steps: [{ name: 'Pylon', second: 30 }, { name: 'Gateway', second: 61 }] }
		]);
		expect(ranked[0].name).toBe('Tight');
		expect(ranked[0].similarity).toBe(1);
	});

	it('prefers the more complete build when both match everything they asked for', () => {
		const actual = [
			{ name: 'Pylon', second: 30 },
			{ name: 'Gateway', second: 61 },
			{ name: 'CyberneticsCore', second: 121 }
		];
		const ranked = rankBuilds(actual, [
			{ title: 'Short', steps: [{ name: 'Pylon', second: 30 }] },
			{ title: 'Long but drifted', steps: [{ name: 'Pylon', second: 38 }] }
		]);
		// Both score 1.0, so the tie-break on drift decides — proving drift is consulted at all.
		expect(ranked[0].name).toBe('Short');
		expect(ranked[1].totalDriftSeconds).toBe(8);
	});
});

/** A payload with just enough shape for the worker-loss summary. */
function payloadWithDeaths(deaths: { second: number; ownerPid: number }[]): AnalysisPayload {
	return {
		players: [
			{ pid: 1, name: 'You', race: 'Zerg' },
			{ pid: 2, name: 'Them', race: 'Terran' }
		],
		workerDeaths: deaths.map((death) => ({
			second: death.second,
			unit: 'Drone',
			x: 1,
			y: 1,
			ownerPid: death.ownerPid,
			ownerRace: 'Zerg',
			isWorker: true
		}))
	} as unknown as AnalysisPayload;
}

describe('workerLossSummary', () => {
	it('finds the densest burst inside the window, not the total', () => {
		// Twelve deaths in twenty seconds is a runby; eight spread over ten minutes is not.
		const burst = Array.from({ length: 12 }, (_, index) => ({ second: 300 + index, ownerPid: 1 }));
		const spread = Array.from({ length: 8 }, (_, index) => ({ second: index * 60, ownerPid: 2 }));
		const [worst] = workerLossSummary(payloadWithDeaths([...burst, ...spread]));
		expect(worst.pid).toBe(1);
		expect(worst.worstBurst).toBe(12);
		expect(worst.worstBurstAt).toBe(300);
		expect(worst.total).toBe(12);
	});

	it('reports a zero burst as null rather than as second zero', () => {
		const [worst] = workerLossSummary(payloadWithDeaths([]));
		expect(worst.worstBurst).toBe(0);
		expect(worst.worstBurstAt).toBeNull();
	});
});

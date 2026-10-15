/**
 * A hardcoded sample build order, for the skeleton only.
 *
 * It exists so the overlay and the post-game view have something real to render before
 * the Python sidecar lands. It is replaced by parsed data in Phase 1 and is deliberately
 * small, so there is no incentive to build on it: 17 steps of a Zergling/Baneling ZvT,
 * as the canonical `supply / time / unit` triple a build order is written in.
 *
 * Times are in-game seconds (see AGENTS.md). The names match what the game reports, so
 * this fixture behaves the same as a parsed order will.
 */

import type { BuildOrder } from './types';

/** A 17-step, 3-base ZvT opening, standing in for parsed data until Phase 1. */
export const sampleBuildOrder: BuildOrder = {
	id: 'sample-zvt-ling-bane',
	title: 'Ling Bane 3-Base (sample)',
	matchup: 'ZvT',
	source: 'user',
	steps: [
		{ supply: 6, timeInGameSeconds: 0, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
		{ supply: 9, timeInGameSeconds: 10, name: 'Overlord', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 10, timeInGameSeconds: 18, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
		{ supply: 14, timeInGameSeconds: 34, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
		{ supply: 14, timeInGameSeconds: 48, name: 'Extractor', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 15, timeInGameSeconds: 60, name: 'Spawning Pool', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 15, timeInGameSeconds: 74, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
		{ supply: 17, timeInGameSeconds: 88, name: 'Extractor', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 17, timeInGameSeconds: 100, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
		{ supply: 19, timeInGameSeconds: 112, name: 'Drone', isWorker: true, isChronoboosted: false, note: null },
		{ supply: 19, timeInGameSeconds: 124, name: 'Queen', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 20, timeInGameSeconds: 136, name: 'Hatchery', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 20, timeInGameSeconds: 150, name: 'Zergling', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 22, timeInGameSeconds: 170, name: 'Hatchery', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 22, timeInGameSeconds: 190, name: 'Zergling x2', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 24, timeInGameSeconds: 220, name: 'Baneling Nest', isWorker: false, isChronoboosted: false, note: null },
		{ supply: 26, timeInGameSeconds: 250, name: 'Zergling Speed', isWorker: false, isChronoboosted: false, note: null }
	]
};

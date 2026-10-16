/**
 * Tests for the automatic view switch.
 *
 * The behaviour that matters is that the app shows the build-order guide while a game is
 * being played and the analysis view at every other moment — including the instant the match
 * ends, which is the transition the user asked for by name. These tests pin the readings the
 * real client produces (recovered from captured transitions, see `game-mode.ts`) rather than
 * the clock thresholds an earlier version of this module guessed at, because a pause and a
 * game end are indistinguishable from the clock and perfectly distinguishable from `/ui`.
 */

import { describe, expect, it } from 'vitest';

import { decideMode } from './game-mode';
import type { LiveState } from './client-api';

/** The in-game reading: the client reports that no UI screen is open. */
const IN_GAME_SCREENS: string[] = [];

/** The post-game reading, as captured from a real client. */
const SCORE_SCREENS = [
	'ScreenBackgroundSC2/ScreenBackgroundSC2',
	'ScreenNavigationSC2/ScreenNavigationSC2',
	'ScreenForegroundSC2/ScreenForegroundSC2',
	'ScreenScore/ScreenScore',
	'ScreenCustom/ScreenCustom'
];

/** A live poll at the given clock reading, with the given screens open. */
function liveAt(clock: number, screens: string[] | null = IN_GAME_SCREENS): LiveState {
	return {
		status: 'live',
		game: { isReplay: false, displayTime: clock, players: [] },
		screens,
		lastOkAt: 1,
		detail: null
	};
}

/** A poll taken while the player is watching a replay. */
function replayAt(clock: number, screens: string[] | null = IN_GAME_SCREENS): LiveState {
	return {
		status: 'replay',
		game: { isReplay: true, displayTime: clock, players: [] },
		screens,
		lastOkAt: 1,
		detail: null
	};
}

/** A poll where the client API could not be reached at all. */
const UNREACHABLE: LiveState = {
	status: 'unavailable',
	game: null,
	screens: null,
	lastOkAt: null,
	detail: 'connection refused'
};

describe('decideMode', () => {
	it('shows the build guide while a game is being played', () => {
		expect(decideMode(liveAt(12))).toBe('overlay');
	});

	it('switches to the analysis view on the score screen', () => {
		// The match is over: this is the transition the user asked the app to make by itself.
		expect(decideMode(liveAt(605, SCORE_SCREENS))).toBe('analysis');
	});

	it('does NOT leave the build guide for a pause', () => {
		// A pause freezes the clock but does not open a screen, so the guide must stay. This is
		// the case the previous clock-stall heuristic got wrong: it read a pause as a game end.
		expect(decideMode(liveAt(300))).toBe('overlay');
		expect(decideMode(liveAt(300))).toBe('overlay');
		expect(decideMode(liveAt(300))).toBe('overlay');
	});

	it('is a pure function of the reading, with no hidden elapsed-time state', () => {
		// The same reading must always produce the same answer. A function that consulted how
		// long a value had been held would be untestable and non-deterministic; the elapsed-time
		// heuristic was removed for exactly this reason.
		expect(decideMode(liveAt(900))).toBe(decideMode(liveAt(900)));
	});

	it('shows the analysis view in the menus', () => {
		expect(decideMode(liveAt(0, ['ScreenHome/ScreenHome']))).toBe('analysis');
	});

	it('shows the analysis view while loading a map, before the game starts', () => {
		expect(decideMode(liveAt(0, ['ScreenLoading/ScreenLoading']))).toBe('analysis');
	});

	it('shows the analysis view for a replay', () => {
		expect(decideMode(replayAt(30))).toBe('analysis');
	});

	it('sends a replay to the analysis view even when its screen list looks in-game', () => {
		// A replay's screen list is the same empty reading as a live game, so the replay check
		// has to come first or a replay would show a build guide over its own controls.
		expect(decideMode(replayAt(30, IN_GAME_SCREENS))).toBe('analysis');
	});

	it('reports no mode when the client cannot be reached', () => {
		expect(decideMode(UNREACHABLE)).toBeNull();
	});

	it('reports no mode when /ui cannot be read, even if /game can', () => {
		// `null` screens means "could not tell", which is not the same as "in a game". Guessing
		// here would flip a live game into the analysis view on a transient `/ui` failure.
		expect(decideMode(liveAt(120, null))).toBeNull();
	});

	it('round-trips overlay -> analysis -> overlay across two games', () => {
		// Playing, score screen, menus, next game: the mode must come back, or a second game
		// would be stuck in analysis from the first game's end.
		expect(decideMode(liveAt(100))).toBe('overlay');
		expect(decideMode(liveAt(200, SCORE_SCREENS))).toBe('analysis');
		expect(decideMode(liveAt(0, ['ScreenHome/ScreenHome']))).toBe('analysis');
		expect(decideMode(liveAt(5))).toBe('overlay');
	});
});

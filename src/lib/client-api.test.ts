/**
 * Unit tests for reading the SC2 client API's payload.
 *
 * The live payload is pinned in a test because it was captured from a **real running game**
 * and contains a surprise that the code originally got wrong: the client reports race as a
 * four-character code (`"Prot"`), not `"Protoss"`. That mistake dropped the opponent and was
 * invisible in a Zerg matchup, so it is exactly the sort of thing a fixture protects.
 */

import { describe, expect, it } from 'vitest';

import { normaliseRace, parseGamePayload } from './client-api';

/** The exact body `GET http://localhost:6119/game` returned mid-game on 2026-10-14. */
const LIVE_PROTOSS_VS_RANDOM_AI = {
	isReplay: false,
	displayTime: 132.0,
	players: [
		{ id: 1, name: 'flashandfoul', type: 'user', race: 'Prot', result: 'Undecided' },
		{ id: 2, name: 'A.I. 1 (Very Easy)', type: 'computer', race: 'random', result: 'Undecided' }
	]
};

describe('normaliseRace', () => {
	it('accepts the four-character codes the live client sends', () => {
		expect(normaliseRace('Prot')).toBe('Protoss');
		expect(normaliseRace('Terr')).toBe('Terran');
		expect(normaliseRace('Zerg')).toBe('Zerg');
	});

	it('accepts full names, in any case', () => {
		expect(normaliseRace('protoss')).toBe('Protoss');
		expect(normaliseRace('TERRAN')).toBe('Terran');
		expect(normaliseRace(' zerg ')).toBe('Zerg');
	});

	it('returns null for random, which cannot be resolved before the game does', () => {
		expect(normaliseRace('random')).toBeNull();
	});

	it('returns null for missing or non-string values', () => {
		expect(normaliseRace(undefined)).toBeNull();
		expect(normaliseRace(null)).toBeNull();
		expect(normaliseRace(7)).toBeNull();
		expect(normaliseRace('')).toBeNull();
	});
});

describe('parseGamePayload', () => {
	it('reads the clock, the replay flag, and the player the client actually names', () => {
		const state = parseGamePayload(LIVE_PROTOSS_VS_RANDOM_AI);
		expect(state).not.toBeNull();
		expect(state?.displayTime).toBe(132);
		expect(state?.isReplay).toBe(false);
		// The Protoss player must survive normalisation — this is the regression.
		expect(state?.players).toEqual([
			{ name: 'flashandfoul', type: 'user', race: 'Protoss', result: 'Undecided' }
		]);
	});

	it('returns null for a payload that is not an object', () => {
		expect(parseGamePayload(null)).toBeNull();
		expect(parseGamePayload('nope')).toBeNull();
	});

	it('treats a missing displayTime as absent rather than zero', () => {
		// Zero would be indistinguishable from "the game just started", which would make the
		// overlay count a build order from the beginning when it has no clock at all.
		const state = parseGamePayload({ isReplay: false, players: [] });
		expect(state?.displayTime).toBeNull();
	});
});

import type { Matchup, Race } from './types';

/**
 * Builds the `ZvP`-style matchup key from a player's own race and the opponent's.
 *
 * This is the key that selects a preset build order, so it is built the same way
 * everywhere rather than string-concatenated at call sites.
 *
 * @param own The race of the player whose point of view the matchup is written from.
 * @param opponent The opposing race.
 * @returns The matchup key, e.g. `ZvP`.
 */
export function matchupOf(own: Race, opponent: Race): Matchup {
	return `${own[0]}v${opponent[0]}` as Matchup;
}

/**
 * Formats a game-clock value the way StarCraft displays it: `m:ss`, or `h:mm:ss` past
 * an hour. Negative and non-finite values render as `--:--` rather than lying.
 *
 * @param secondsInGame Seconds on the game clock (NOT wall-clock seconds).
 * @returns A display string such as `9:35`.
 */
export function formatGameTime(secondsInGame: number): string {
	if (!Number.isFinite(secondsInGame) || secondsInGame < 0) return '--:--';
	const total = Math.floor(secondsInGame);
	const s = total % 60;
	const m = Math.floor(total / 60) % 60;
	const h = Math.floor(total / 3600);
	const pad = (n: number) => n.toString().padStart(2, '0');
	if (h > 0) return `${h}:${pad(m)}:${pad(s)}`;
	return `${m}:${pad(s)}`;
}

/**
 * Reports that the game clock advances at the same rate as real time.
 *
 * From Legacy of the Void onward the SC2 game clock runs **1:1** with wall-clock time at
 * `Faster`, the ladder and custom-game default. The older "Blizzard time" offset (where one
 * in-game second was about 1.4 real seconds) was a Wings of Liberty / Heart of the Swarm
 * behaviour and no longer applies. Confirmed two ways on this machine: the live client API
 * reported `displayTime` advancing 309 s over 304 s of wall clock (~1:1, over a 5-minute
 * sample), and replay `Duration` in `replay.gamemetadata.json` matches `frames / 16` to
 * within 1% across 722 replays while differing from the in-game clock by ~1.4x — which shows
 * the game *clock* is not 1.4x real time, and that sc2reader's `replay.length` is the field
 * that is wrong, not the clock.
 *
 * There is deliberately no conversion helper here. A function taking a speed multiplier would
 * invite the assumption that the multiplier is ever something other than 1, which is how the
 * 1.4 factor came to be applied in the first place. Anywhere a real-time figure is needed,
 * it equals the game-clock figure, with pauses being the only difference between them.
 */
export const GAME_CLOCK_IS_REAL_TIME = true;

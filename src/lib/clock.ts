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
 * Converts the game clock to wall-clock seconds at a given game-speed multiplier.
 *
 * The replay clock is game time and never advances while a game is paused, so this is
 * only meaningful once the game has finished (or for reasoning about a recording).
 * Ladder and custom games default to `Faster`, where the factor is 1.4 — measured across
 * 722 replays (531 at exactly 1.40, 95 at 1.41).
 *
 * @param secondsInGame Seconds on the game clock.
 * @param speedMultiplier Game-speed factor; 1.4 is `Faster`, the ladder default.
 * @returns Wall-clock seconds.
 */
export function gameToRealSeconds(secondsInGame: number, speedMultiplier = 1.4): number {
	return secondsInGame * speedMultiplier;
}

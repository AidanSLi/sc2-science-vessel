import type { BuildMatchup, BuildOrder, BuildStep } from './types';

/**
 * Where a build order has got to, given a game clock reading. Pure and total, so the
 * overlay and the post-game comparison can share one implementation.
 */
export interface BuildProgress {
	/** Index of the last step whose time has passed, or -1 before the first step. */
	lastDueIndex: number;
	/** Index of the step the player is working toward, or -1 when the build is done. */
	nextIndex: number;
	/** Steps whose time passed with no time left to have done them; see `missedSteps`. */
	missedIndexes: number[];
}

/**
 * Advances a build order to a game-clock reading.
 *
 * Crucially the caller passes the **game clock**, never elapsed wall-clock time. Because
 * the game freezes its own clock while paused, passing `displayTime` from the client API
 * is what makes the guide pause with the game, for free and exactly (ADR 0004). A
 * wall-clock timer would keep running through a pause and drift.
 *
 * A step counts as "missed" once the clock is more than `missToleranceInGameSeconds` past
 * it, because a player realistically executes a step a few seconds late. The default of
 * 10 s matches the resolution of the stats events the numbers were checked against, and
 * is the tolerance the feature list asks for.
 *
 * @param steps Build steps, ordered by `timeInGameSeconds` ascending.
 * @param timeInGameSeconds Current game clock in seconds.
 * @param missToleranceInGameSeconds Grace period before a step counts as missed.
 * @returns How far through the build the clock is.
 */
export function advance(
	steps: readonly BuildStep[],
	timeInGameSeconds: number,
	missToleranceInGameSeconds = 10
): BuildProgress {
	let lastDueIndex = -1;
	const missedIndexes: number[] = [];

	for (let i = 0; i < steps.length; i++) {
		const step = steps[i];
		if (step.timeInGameSeconds > timeInGameSeconds) break;
		lastDueIndex = i;
		// The last step due is always the one being worked on, never a miss.
		if (timeInGameSeconds - step.timeInGameSeconds > missToleranceInGameSeconds) {
			missedIndexes.push(i);
		}
	}

	const nextIndex = lastDueIndex + 1 < steps.length ? lastDueIndex + 1 : -1;
	// The step being worked on is not missed, so drop it from the miss list.
	const workingIndex = missedIndexes[missedIndexes.length - 1];
	if (workingIndex === lastDueIndex) missedIndexes.pop();

	return { lastDueIndex, nextIndex, missedIndexes };
}

/**
 * Groups a set of build orders by matchup so a preset can be picked without scanning.
 *
 * @param builds Build orders to index.
 * @returns A map from matchup to the builds for it, each list keeping input order.
 */
export function indexByMatchup(
	builds: readonly BuildOrder[]
): Partial<Record<BuildMatchup, BuildOrder[]>> {
	const out: Partial<Record<BuildMatchup, BuildOrder[]>> = {};
	for (const build of builds) {
		(out[build.matchup] ??= []).push(build);
	}
	return out;
}

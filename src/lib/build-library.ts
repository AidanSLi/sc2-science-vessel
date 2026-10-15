/**
 * The bundled build-order library, read from the SpawningTool snapshot.
 *
 * The snapshot (`builds.json`) is generated offline by `tools/refresh_build_snapshot.py`, which
 * is the only component allowed to use the network (ADR 0002). Bundling it means build-order
 * comparison works with no connection, which is the app's premise; refreshing is an explicit
 * user action, and every failure path keeps the snapshot that is already there rather than
 * leaving an empty library that looks like "no builds match your game".
 *
 * The snapshot's step shape is `{supply, second, name, category, note}`. Two normalisations
 * happen here, and both are corrections to how SpawningTool writes its data rather than
 * preferences:
 *
 * 1. `second` is already on the game clock. The scraper converts the site's display time, which
 *    is on the pre-LotV `22.4` fps scale; see `tools/refresh_build_snapshot.py` and the
 *    `spawningtool` note in AGENTS.md. Nothing here divides by anything.
 * 2. Names read `Drone x2`, `Extractor x2` — SpawningTool merges repeated actions into one row
 *    and puts the count in the name. Left alone, every comparison against a replay would miss
 *    every doubled step, because the tracker stream emits `Drone` twice. The count is split out
 *    into repeats so a merged row matches the two steps it stands for.
 *
 * The builder's own `note` is carried through as well. It is not decoration: 19 steps in the
 * snapshot are named `Action` with the entire instruction in the note, so a build order without
 * notes would show those steps as a meaningless `Action` at a time.
 */

import type { BuildMatchup, BuildOrder, BuildStep } from './types';

/** One step as the snapshot stores it, before normalisation. */
interface SnapshotStep {
	supply: number | null;
	second: number;
	name: string;
	category: string;
	note: string;
}

/** One build as the snapshot stores it. */
interface SnapshotBuild {
	id: number;
	name: string;
	matchup: string | null;
	steps: SnapshotStep[];
}

/** The snapshot file's shape. */
interface Snapshot {
	source: string;
	generatedAt: string;
	builds: SnapshotBuild[];
}

/** The bundled snapshot. Imported statically so it is inlined into the bundle, not fetched. */
import snapshot from './builds.json';

/** Every matchup the snapshot uses, as the app spells them. */
const KNOWN_MATCHUPS = new Set(['TvT', 'TvP', 'TvZ', 'TvX', 'PvT', 'PvP', 'PvZ', 'PvX', 'ZvT', 'ZvP', 'ZvZ', 'ZvX']);

/** Matches a trailing repeat count, e.g. the ` x2` in `Drone x2`. */
const REPEAT_SUFFIX = /\s+x(\d+)$/;

/** Matches the chronoboost annotation SpawningTool appends to a step's name. */
const CHRONO_SUFFIX = /\s*\(Chrono Boost\)\s*$/;

/**
 * Splits a step name into its unit name and how many times it repeats.
 *
 * @param raw The name as the snapshot stores it, e.g. `Marine x3`.
 * @returns The bare name and the repeat count, which is 1 when the row is not merged.
 */
export function splitRepeats(raw: string): { name: string; repeats: number } {
	const match = REPEAT_SUFFIX.exec(raw.trim());
	if (!match) return { name: raw.trim(), repeats: 1 };
	const repeats = Number.parseInt(match[1], 10);
	return { name: raw.trim().slice(0, match.index).trim(), repeats: repeats > 0 ? repeats : 1 };
}

/**
 * Converts one snapshot build into the app's `BuildOrder`.
 *
 * @param build The snapshot record.
 * @returns A build order, with merged rows expanded into one step per repetition.
 */
export function toBuildOrder(build: SnapshotBuild): BuildOrder {
	const matchup = (build.matchup ?? '') as BuildMatchup;
	const steps: BuildStep[] = [];
	for (const step of build.steps) {
		// Order matters, and the data decides it: SpawningTool nests a merged row's count *inside*
		// the chrono annotation — the one merged-and-chronoed row in the bundled snapshot reads
		// `Probe x2 (Chrono Boost)`. So the suffix is stripped first, and the count is then the last
		// thing in the string. Stripping in the other order leaves the count unmatched, because the
		// repeat pattern is anchored at the end of the string and the string ends in `(Chrono Boost)`.
		// The unmerged case (`Action (Chrono Boost)`) works the same way with `repeats` left at 1.
		const chronoboosted = CHRONO_SUFFIX.test(step.name);
		const { name, repeats } = splitRepeats(step.name.replace(CHRONO_SUFFIX, ''));
		for (let i = 0; i < repeats; i += 1) {
			steps.push({
				supply: step.supply ?? 0,
				timeInGameSeconds: step.second,
				name,
				isWorker: step.category === 'Worker',
				isChronoboosted: chronoboosted,
				// A merged row's note describes the group (`工蜂, 工蜂`, `Cycle 1...`), so every
				// repetition carries it rather than only the first, which would make the note vanish
				// from all but one step of a `x3` row.
				note: step.note.trim() === '' ? null : step.note.trim()
			});
		}
	}
	/**
	 * Sorts the steps onto the clock, stably.
	 *
	 * A guide is ordered by time by definition, and both `advance` (which stops at the first
	 * future step) and `compareToBuild` (which matches greedily in order) depend on that. The
	 * site does not guarantee it: at least one curated build in the bundled snapshot lists a
	 * `Raven` at 278s before a `Refinery` at 260s, and taken as-is that step would never be
	 * reached by the overlay and would never be matchable. Sorting duplicates keeps the author's
	 * order for simultaneous steps, which is the order SpawningTool displays.
	 */
	steps.sort((a, b) => a.timeInGameSeconds - b.timeInGameSeconds);
	return {
		id: `spawningtool-${build.id}`,
		title: build.name,
		matchup,
		source: 'bundled',
		author: 'SpawningTool',
		steps
	};
}

/**
 * Every build in the bundled snapshot.
 *
 * Built once at module load: the snapshot is a few hundred kilobytes of already-parsed JSON, and
 * re-normalising it per render would be work with no purpose.
 */
export const bundledBuilds: BuildOrder[] = (snapshot as Snapshot).builds.map(toBuildOrder);

/** When the bundled snapshot was generated, for the UI to show its age. */
export const bundledSnapshotDate: string = (snapshot as Snapshot).generatedAt;

/** The snapshot's upstream, so provenance is visible rather than implied. */
export const bundledSnapshotSource: string = (snapshot as Snapshot).source;

/**
 * Reports whether a snapshot matchup string is one the app understands.
 *
 * The snapshot contains `X` matchups (`TvX`, meaning "against any race"), which are real
 * SpawningTool categories but are not presets for a specific matchup, so callers that need an
 * exact matchup must filter them out rather than treating `TvX` as one.
 *
 * @param value The matchup string.
 * @returns True when it is a matchup the app recognises.
 */
export function isKnownMatchup(value: string): boolean {
	return KNOWN_MATCHUPS.has(value);
}

/**
 * Converts a build order's steps into the shape `compareToBuild` matches against.
 *
 * The two step types are deliberately different: `BuildStep` is a *guide* (`timeInGameSeconds`,
 * a `BuildOrder`, a source) while the comparison's steps are the minimal `{name, second, supply}`
 * the matcher needs. Separating them keeps the matcher usable for both a bundled guide and a
 * replay's measured steps without either having to adopt the other's fields; this function is the
 * one place the translation happens, mirroring `sidecar/sc2sv/builds.py:reference_steps`.
 *
 * @param build A build order.
 * @returns Steps in matcher shape, time-ordered.
 */
export function referenceSteps(build: BuildOrder): { name: string; second: number; supply: number | null; note: string | null }[] {
	return build.steps.map((step) => ({
		name: step.name,
		second: step.timeInGameSeconds,
		supply: step.supply ?? null,
		note: step.note
	}));
}

/**
 * Selects the bundled builds usable as a reference for a matchup.
 *
 * Includes the `X` variants: a ZvT game has only a handful of exact `ZvT` builds in a snapshot
 * of this size, and a `ZvX` build's opener is valid against Terran. They are sorted after the
 * exact matches because a build written for a specific matchup is the better reference.
 *
 * @param matchup The game's matchup, e.g. `ZvT`.
 * @returns Candidate builds, exact matches first.
 */
export function buildsForMatchup(matchup: string): BuildOrder[] {
	const own = matchup.slice(0, 1).toUpperCase();
	const exact: BuildOrder[] = [];
	const wildcard: BuildOrder[] = [];
	for (const build of bundledBuilds) {
		if (build.matchup === matchup) exact.push(build);
		else if (build.matchup === `${own}vX`) wildcard.push(build);
	}
	return [...exact, ...wildcard];
}

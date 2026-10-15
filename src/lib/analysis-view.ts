/**
 * Turns analysis payloads into what the views draw.
 *
 * Everything here is pure and payload-in/marks-out, so the decisions that are easy to get
 * subtly wrong — which colour identifies a player, where a schematic map puts a battle, which
 * steps count as discrepancies — are testable without rendering anything. The page components
 * remain about layout.
 *
 * The colour rule is the load-bearing one. A player is identified by **race colour plus a
 * dash pattern for the second player of that race**, never by index alone: a PvP has two
 * Protoss, and if they shared one gold line the resource-differential panel would be
 * unreadable exactly when it matters most. Mutating the stroke so a same-race pair stays
 * distinguishable is the least surprising way to do that.
 */

import type {
	AnalysisPayload,
	AnalysisPlayer,
	BuildOrderStep,
	DeathLocation,
	EnergyTimeline,
	SupplyBlock,
	TrackedStep
} from './analysis';
import type { Marker } from './components/TimelineChart.svelte';
import type { Race } from './types';

/**
 * Colour token for a race.
 *
 * @param race The player's race.
 * @returns A token name from `app.css`, never a raw colour.
 */
export function raceColorToken(race: Race | string): string {
	switch (String(race).toLowerCase()) {
		case 'terran':
			return 'race-terran';
		case 'protoss':
			return 'race-protoss';
		case 'zerg':
			return 'race-zerg';
		default:
			return 'neutral';
	}
}

/**
 * Colour token for a player, disambiguating a same-race pair.
 *
 * A mirror matchup would otherwise draw both players in one colour. The second player of a
 * race still gets that race's hue — so the panel keeps meaning "the Zerg line" — but the
 * reference line's dashes separate them. Returning the dash decision alongside the token keeps
 * the pairing in one place rather than having every caller recompute it.
 *
 * @param player The player to colour.
 * @param allPlayers Every player in the game, in payload order.
 * @returns The token, plus whether this player's line should be dashed.
 */
export function playerStyle(
	player: Pick<AnalysisPlayer, 'pid' | 'race'>,
	allPlayers: readonly Pick<AnalysisPlayer, 'pid' | 'race'>[]
): { colorToken: string; dash: string | undefined } {
	const sameRace = allPlayers.filter((other) => other.race === player.race);
	// A player is "second of their race" only when their race is actually shared; a lone Zerg in
	// a PvZ is the first and only one even if the payload lists them second.
	const indexAmongRace = sameRace.findIndex((other) => other.pid === player.pid);
	return {
		colorToken: raceColorToken(player.race),
		dash: indexAmongRace > 0 ? '5 3' : undefined
	};
}

/**
 * Converts supply blocks into chart markers.
 *
 * Each block becomes one band. When the player was still capped at the end of the game the band
 * has no known end, so it is drawn to the end of the game and flagged `uncertain`: leaving it
 * out would hide a real problem, and drawing it as a definite span would invent an end time.
 *
 * @param blocks Supply blocks from one player.
 * @param colorToken Colour for the band.
 * @param lengthSeconds The game's length, used to close an open-ended block.
 * @returns Markers for a timeline chart.
 */
export function supplyBlockMarkers(
	blocks: readonly SupplyBlock[],
	colorToken: string,
	lengthSeconds: number
): Marker[] {
	return blocks.map((block) => {
		const from = block.sometimeAfter;
		// `clearedBy` is null either because the game ended while capped or because the last
		// sample was still at cap, and in both cases the band runs to the end of the game. Using
		// the game length rather than skipping the block keeps a 20-minute supply block visible.
		const cleared = block.clearedBy;
		const to = block.openEnded || cleared === null ? lengthSeconds : cleared;
		const label =
			`Supply blocked, at cap sometime after ${Math.round(from)}s` +
			(block.openEnded || cleared === null
				? ' and never cleared'
				: `, cleared by ${Math.round(cleared)}s (${Math.round(block.minSeconds)}-${Math.round(block.maxSeconds)}s)`);
		return {
			from,
			to: Math.max(from, to),
			label,
			colorToken,
			// The exact start is unknown because stats samples are 10 s apart, so the band is
			// drawn faintly rather than pretending to a precision the replay does not contain.
			uncertain: true
		};
	});
}

/**
 * Converts one player's supply blocks into points for a "time capped" chart.
 *
 * A band alone cannot be compared against the opponent. These samples give each block a
 * plottable value — the verified minimum seconds capped — placed at the block's midpoint, so a
 * long block reaches higher than a short one and the two players can share one axis.
 *
 * @param blocks Supply blocks from one player.
 * @returns Parallel x and y arrays, using the block's minimum verified duration.
 */
export function supplyBlockDurations(blocks: readonly SupplyBlock[]): { x: number[]; y: number[] } {
	return {
		// An open-ended block has no clear second, so the midpoint of its verified bounds is the
		// best available position — placing it at the end of the game would draw a late-game spike
		// that the data does not support.
		x: blocks.map((block) => {
			const end = block.clearedBy ?? block.sometimeAfter + block.maxSeconds;
			return (block.sometimeAfter + end) / 2;
		}),
		y: blocks.map((block) => Math.round(block.minSeconds))
	};
}

/**
 * Converts an energy timeline into chart markers.
 *
 * Gaps become bands and casts become zero-width bands. Both are drawn in the caller's colour so
 * the panel reads as one ability's uptime rather than as two unrelated overlays.
 *
 * @param timeline One ability's timeline.
 * @param colorToken Colour for the marks.
 * @returns Markers for a timeline chart.
 */
export function energyMarkers(timeline: EnergyTimeline, colorToken: string): Marker[] {
	const marks: Marker[] = timeline.gaps
		// A gap shorter than the ideal cycle is on schedule, not a miss.
		.filter((gap) => gap.idleSeconds > 0)
		.map((gap) => ({
			from: gap.from,
			to: gap.to,
			label: `${timeline.kind} idle ${Math.round(gap.idleSeconds)}s`,
			colorToken,
			// The cast times themselves are exact; what is inferred is only that the base was
			// "available" across the whole span, since energy can be banked while capped.
			uncertain: true
		}));
	for (const second of timeline.seconds) {
		marks.push({ from: second, to: second, label: `${timeline.kind} cast at ${Math.round(second)}s`, colorToken });
	}
	return marks;
}

/**
 * Turns tracked structure steps into chart markers.
 *
 * The tracker stream already timestamps every structure, so an "uptime" panel can be drawn as
 * one tick per structure rather than as a derived percentage. That is the honest presentation:
 * the replay records when a Forge started and when it finished, but it does not record which
 * upgrades were researched and which were merely available, so any single "Forge uptime"
 * number would be inventing the thing the player actually wants to know (were you late to
 * +1 attack, or did you spend the whole game choosing not to upgrade). The ticks let the
 * reader answer that by eye against the upgrade list beside them.
 *
 * @param steps Structure steps from one player, in game seconds.
 * @param colorToken Colour for the marks.
 * @returns One zero-width marker per structure.
 */
export function stepMarkers(steps: readonly TrackedStep[], colorToken: string): Marker[] {
	return steps.map((step) => ({
		from: step.second,
		to: step.second,
		label: `${step.name} at ${Math.round(step.second)}s`,
		colorToken
	}));
}

/**
 * Turns the discrepancies in a build comparison into chart markers.
 *
 * Only matched steps get a mark: a step the player never made at all is already reported in the
 * missing list, and drawing it on a timeline would imply it happened somewhere. The mark sits
 * at the second the player actually hit the step, so a run of marks drifting right is visible
 * at a glance.
 *
 * @param comparison A comparison from `compareToBuild`.
 * @param colorToken Colour for the marks.
 * @returns One marker per matched step, at its actual second.
 */
export function discrepancyMarkers(comparison: BuildComparison, colorToken: string): Marker[] {
	return comparison.matched.map((entry) => ({
		from: entry.actualSecond,
		to: entry.actualSecond,
		label:
			`${entry.name}: ${entry.verdict} by ${Math.abs(entry.deltaSeconds).toFixed(1)}s ` +
			`(planned ${Math.round(entry.expectedSecond)}s, you ${Math.round(entry.actualSecond)}s)`,
		colorToken
	}));
}

/**
 * Projects replay coordinates into a unit square for the schematic battle map.
 *
 * The replay records no terrain: rendering the actual map needs the map file, which is not
 * available offline. What *is* exact is the relative placement of the deaths, so the schematic
 * keeps the aspect ratio of the area actually used and says so in the UI. Stretching each axis
 * to fill the box would make two attacks on opposite sides of the map look like two attacks on
 * opposite corners, which is a lie the picture is not allowed to tell.
 *
 * The larger of the two spans is scaled to fill the box, so the shape of the fighting is
 * preserved; the projection never distorts one axis relative to the other.
 *
 * @param points Every death location with coordinates.
 * @returns A function from replay `(x, y)` to normalised `[0, 1]` coordinates, plus the data
 *     bounds it was built from.
 */
export function projectToUnitSquare(
	points: readonly Pick<DeathLocation, 'x' | 'y'>[]
): {
	project: (x: number, y: number) => [number, number];
	bounds: { minX: number; maxX: number; minY: number; maxY: number } | null;
} {
	const xs = points.map((point) => point.x).filter((x): x is number => x !== null);
	const ys = points.map((point) => point.y).filter((y): y is number => y !== null);
	if (xs.length === 0 || ys.length === 0) {
		return { project: () => [0.5, 0.5], bounds: null };
	}
	const minX = Math.min(...xs);
	const maxX = Math.max(...xs);
	const minY = Math.min(...ys);
	const maxY = Math.max(...ys);
	// A single point, or a perfectly straight line of them, has no span to normalise against.
	const spanX = maxX - minX || 1;
	const spanY = maxY - minY || 1;
	const span = Math.max(spanX, spanY);
	return {
		project: (x, y) => [(x - minX) / span, (y - minY) / span],
		bounds: { minX, maxX, minY, maxY }
	};
}

/** One player's build compared against a reference build. */
export interface BuildComparison {
	name: string;
	similarity: number;
	totalDriftSeconds: number;
	matched: {
		name: string;
		expectedSecond: number;
		actualSecond: number;
		deltaSeconds: number;
		verdict: string;
		/** The reference's own note for this step, if it had one. */
		note?: string | null;
	}[];
	missing: { name: string; expectedSecond: number; supply?: number | null; note?: string | null }[];
	extra: { name: string; second: number }[];
}

/**
 * Compares a build order against a reference, mirroring the sidecar's matching.
 *
 * This is a TypeScript port of `sidecar/sc2sv/analyze.py:compare_to_build` and exists so the
 * analysis page can re-run a comparison the moment the user picks a different reference build,
 * without a round trip to the sidecar. The two implementations must agree, so
 * `src/lib/analysis-view.test.ts` asserts they produce the same verdicts for the same inputs;
 * a drift between them would show up as a build that scores well in the app and badly in the
 * CLI.
 *
 * Matching is greedy and in order, exactly as in the sidecar: a build order is a sequence, and
 * letting a late `Gateway` claim an earlier reference `Gateway` that was already consumed would
 * report a false hit.
 *
 * @param actual The replay's steps for one player, time-ordered.
 * @param reference The reference build's steps, time-ordered.
 * @param tolerance Seconds within which a step counts as hitting its target.
 * @returns The comparison result, with the reference's notes carried through so the UI can show
 *     what a step was actually for — the snapshot has 19 steps named `Action` whose entire
 *     instruction is in the note.
 */
export function compareToBuild(
	actual: readonly Pick<BuildOrderStep, 'name' | 'second'>[],
	reference: readonly (Pick<BuildOrderStep, 'name' | 'second'> & { supply?: number | null; note?: string | null })[],
	tolerance = 10
): BuildComparison {
	const used = actual.map(() => false);
	const matched: BuildComparison['matched'] = [];
	const missing: BuildComparison['missing'] = [];
	for (const target of reference) {
		let claim = -1;
		for (let i = 0; i < actual.length; i += 1) {
			if (used[i] || actual[i].name !== target.name) continue;
			if (Math.abs(actual[i].second - target.second) <= tolerance) {
				claim = i;
				break;
			}
		}
		if (claim < 0) {
			missing.push({
				name: target.name,
				expectedSecond: target.second,
				supply: target.supply ?? null,
				note: target.note ?? null
			});
			continue;
		}
		used[claim] = true;
		const delta = Math.round((actual[claim].second - target.second) * 10) / 10;
		matched.push({
			name: target.name,
			expectedSecond: target.second,
			actualSecond: actual[claim].second,
			deltaSeconds: delta,
			verdict: Math.abs(delta) <= 2 ? 'on time' : delta < 0 ? 'early' : 'late',
			note: target.note ?? null
		});
	}
	const extra = actual
		.map((step, index) => ({ step, index }))
		.filter(({ index }) => !used[index])
		.map(({ step }) => ({ name: step.name, second: step.second }));
	const referenceCount = matched.length + missing.length;
	return {
		name: '',
		similarity: referenceCount === 0 ? 1 : matched.length / referenceCount,
		totalDriftSeconds: Math.round(matched.reduce((total, entry) => total + Math.abs(entry.deltaSeconds), 0) * 10) / 10,
		matched,
		missing,
		extra
	};
}

/**
 * Ranks named reference builds by how well they match a player's actual build.
 *
 * Ties break on total timing drift: two builds can match the same number of steps while one was
 * followed much more tightly, and the tighter one is the more useful answer.
 *
 * @param actual The player's build order.
 * @param candidates Named reference builds.
 * @param tolerance Matching tolerance in seconds.
 * @returns The candidates, best first, each carrying its comparison.
 */
export function rankBuilds(
	actual: readonly Pick<BuildOrderStep, 'name' | 'second'>[],
	candidates: readonly {
		title: string;
		steps: readonly (Pick<BuildOrderStep, 'name' | 'second'> & { supply?: number | null; note?: string | null })[];
	}[],
	tolerance = 10
): BuildComparison[] {
	return candidates
		.map((candidate) => ({ ...compareToBuild(actual, candidate.steps, tolerance), name: candidate.title }))
		.sort((a, b) => b.similarity - a.similarity || a.totalDriftSeconds - b.totalDriftSeconds);
}

/**
 * Finds a player in a payload by pid.
 *
 * @param payload The analysis payload.
 * @param pid The player id.
 * @returns The player, or undefined when absent.
 */
export function playerById(payload: AnalysisPayload, pid: number): AnalysisPlayer | undefined {
	return payload.players.find((player) => player.pid === pid);
}

/**
 * Summarises what happened to each player's workers, for the runby verdict.
 *
 * The feature the list asks for is "vulnerability to runbys, checked by worker deaths". A raw
 * death count is not that — losing 20 workers across a 30-minute game of constant fighting is
 * normal, while losing 12 in one minute to a runby is the thing worth flagging. So this groups
 * worker deaths into bursts and reports the worst one.
 *
 * @param payload The analysis payload.
 * @param windowSeconds The window a burst is measured over.
 * @returns Per-player worker deaths and the worst burst found, worst first.
 */
export function workerLossSummary(
	payload: AnalysisPayload,
	windowSeconds = 60
): { pid: number; name: string; total: number; worstBurst: number; worstBurstAt: number | null }[] {
	const byOwner = new Map<number, number[]>();
	for (const death of payload.workerDeaths) {
		if (death.ownerPid === null) continue;
		const list = byOwner.get(death.ownerPid) ?? [];
		list.push(death.second);
		byOwner.set(death.ownerPid, list);
	}
	return payload.players
		.map((player) => {
			const times = (byOwner.get(player.pid) ?? []).sort((a, b) => a - b);
			let worstBurst = 0;
			let worstBurstAt: number | null = null;
			for (let i = 0; i < times.length; i += 1) {
				// Count deaths in the window starting at each death, which finds the densest cluster
				// without assuming it lines up with a round number.
				const count = times.filter((time) => time >= times[i] && time <= times[i] + windowSeconds).length;
				if (count > worstBurst) {
					worstBurst = count;
					worstBurstAt = times[i];
				}
			}
			return {
				pid: player.pid,
				name: player.name,
				total: times.length,
				worstBurst,
				worstBurstAt: worstBurst > 0 ? worstBurstAt : null
			};
		})
		.sort((a, b) => b.worstBurst - a.worstBurst || b.total - a.total);
}

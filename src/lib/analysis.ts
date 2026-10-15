/**
 * The render-ready shapes the Python sidecar returns, and the calls that fetch them.
 *
 * These types mirror `sidecar/sc2sv/analyze.py` exactly. They are declared here rather than
 * generated because the payload is small and the sidecar is in this repo: a mismatch shows up
 * as a failing test (`tests/test_payload_contract.py`) rather than as a silent `undefined` in
 * a chart. Do not add a field here that the sidecar does not actually send — the Rust command
 * returns the sidecar's JSON verbatim, so inventing a field produces `undefined` that only
 * surfaces as a blank chart.
 *
 * Every `second` value is on the **game clock** (`frames / 16`), which since Legacy of the
 * Void runs 1:1 with real time at the standard `Faster` speed. See AGENTS.md.
 */

import { isTauri } from './client-api';
import type { Race } from './types';

/** A point on the worker graph, in game seconds. */
export interface WorkerPoint {
	second: number;
	workers: number;
}

/** A worker-count drop, which is what a runby or a lost fight looks like from the economy side. */
export interface WorkerDrop {
	second: number;
	from: number;
	to: number;
	lost: number;
	/**
	 * The game second the sample interval containing this drop began.
	 *
	 * `PlayerStatsEvent` is sampled every 10 game seconds (occasionally 5), so the true second
	 * of a drop is somewhere in `[sometimeAfter, second]`. The UI must show it as a range or a
	 * tick, never as a precise instant — claiming sub-10-second precision would be inventing
	 * data the replay does not contain.
	 */
	sometimeAfter: number;
}

/**
 * How long one player spent supply-blocked.
 *
 * A block is an interval, not a duration, because the samples cannot pin its endpoints: the
 * true start is in `[sometimeAfter, …]` and the true end is in `[…, clearedBy]`. `minSeconds`
 * counts only the at-cap samples observed; `maxSeconds` assumes the cap was reached immediately
 * after the previous sample and cleared immediately before the next. The UI should present the
 * pair as a range — quoting either one alone overstates its confidence.
 */
export interface SupplyBlock {
	sometimeAfter: number;
	/** The second the block was observed clear, or null when the game ended while still capped. */
	clearedBy: number | null;
	minSeconds: number;
	maxSeconds: number;
	/** True when the block was still open at the end of the game. */
	openEnded: boolean;
}

/** One battle: what each side lost and killed in it. */
export interface Engagement {
	/** The game second the loss counters changed on. */
	second: number;
	/** The previous sample's second, so the interval containing the fight is explicit. */
	after: number;
	lost: number;
	killed: number;
	/** Signed from this player's point of view; positive means the trade favoured them. */
	net: number;
}

/**
 * One step of an actual build order as the tracker stream recorded it.
 *
 * Used for the build order *and* for the tech, expansion and resource-structure timelines.
 * Those four are the same record filtered by name, so they share one type rather than three
 * near-identical ones that could drift apart one field at a time.
 */
export interface TrackedStep {
	second: number;
	name: string;
	isWorker: boolean;
	isStructure: boolean;
	/** Map coordinates, absent for units the replay does not position. */
	x: number | null;
	y: number | null;
}

/** A step of the guide-shaped build order from `spawningtool`. */
export interface BuildOrderStep {
	name: string;
	/** Game seconds. Recomputed from the raw frame by the sidecar — see `sidecar/sc2sv/builds.py`. */
	second: number;
	supply: number | null;
}

/** One player's full metric set. */
export interface AnalysisPlayer {
	pid: number;
	name: string;
	race: Race;
	team: number;
	result: string;
	isHuman: boolean;
	workers: { seconds: number[]; workers: number[]; drops: WorkerDrop[] };
	supplyBlocks: SupplyBlock[];
	resources: { seconds: number[]; differential: number[] };
	engagements: Engagement[];
	/** Unit type name to count. Includes the units the player made, not map scenery. */
	composition: Record<string, number>;
	upgrades: { second: number; name: string }[];
	tech: TrackedStep[];
	expansions: TrackedStep[];
	resourceStructures: TrackedStep[];
	/** The opening, which the UI shows as a short list rather than a full timeline. */
	firstStructures: TrackedStep[];
	buildOrder: BuildOrderStep[];
}

/** A unit death, with the position needed to plot it on the map. */
export interface DeathLocation {
	second: number;
	unit: string;
	x: number | null;
	y: number | null;
	ownerPid: number | null;
	ownerRace: string | null;
	isWorker: boolean;
}

/** One stretch where an energy ability was available and unused at a single base. */
export interface EnergyGap {
	/** The town hall's map position, or null when the cast could not be attributed to one. */
	base: [number, number] | null;
	from: number;
	to: number;
	idleSeconds: number;
}

/**
 * How long one energy ability sat unused.
 *
 * `missedSeconds` is downtime measured against the **energy-limited** cycle — the time the
 * ability's cost takes to regenerate — not against its effect duration. Those differ (an
 * inject locks a hatchery for 29s but costs 25 energy, which regenerates in ~31.7s), and the
 * binding constraint is the energy. See `ENERGY_PER_SECOND` in `sidecar/sc2sv/metrics.py`.
 *
 * The replay records neither banked energy nor how many queens were alive, so this is a
 * lower bound on wasteful downtime, not a claim about a player's energy total.
 */
export interface EnergyTimeline {
	/** `inject`, `mule` or `chrono`. */
	kind: string;
	idealSeconds: number;
	/** Town halls counted across every player of the race, so a mirror matchup reports both. */
	bases: number;
	/** Cast times, in game seconds. */
	seconds: number[];
	gaps: EnergyGap[];
	missedSeconds: number;
	/**
	 * Per-player breakdown, present when the replay contains a player of this ability's race.
	 *
	 * Exists because a mirror matchup fields two players of the same race, and one blended
	 * `missedSeconds` cannot say whose macro was idle — on a real 1v1 ZvZ, measuring each
	 * player against their own town halls gave 2578.7 s where pooling them gave 2116.5 s.
	 *
	 * Sorted by pid. Empty when no player of the race was in the game, in which case
	 * `missedSeconds` is 0 because the ability does not exist rather than because it was used.
	 */
	perPlayer?: { pid: number | null; bases: number; missedSeconds: number }[];
}

/** Everything the analysis view renders for one replay. */
export interface AnalysisPayload {
	path: string;
	build: number;
	release: string;
	gameType: string;
	map: string;
	speed: string;
	matchup: string;
	date: string;
	isLadder: boolean;
	lengthSeconds: number;
	mmr: number[];
	players: AnalysisPlayer[];
	deaths: DeathLocation[];
	workerDeaths: DeathLocation[];
	energy: { inject: EnergyTimeline; mule: EnergyTimeline; chrono: EnergyTimeline };
}

/** One replay's header facts, as the library scan reports them. */
export interface ReplayHeaderRecord {
	path: string;
	build: number;
	release: string;
	game_type: string;
	map_name: string;
	speed: string;
	frames: number;
	lengthSeconds: number;
	matchup: string;
	is_ladder: boolean;
	date: string | null;
	mmr: number[];
	winners: string[];
	players: { pid: number; name: string; race: string; team: number; result: string }[];
	error: string | null;
}

/**
 * A message describing why a call could not run, in terms a player can act on.
 *
 * The distinction that matters is "the desktop app is needed" versus "something went wrong":
 * a browser cannot spawn a local process, so in a browser every sidecar call is unavailable by
 * design and must not be reported as a fault. That mirrors the clock's honest-failure handling
 * in `client-api.ts`.
 */
export function sidecarUnavailableReason(): string | null {
	if (isTauri()) return null;
	return 'Reading replays needs the desktop app; a browser cannot run the local analyser.';
}

/** Thrown when a sidecar call fails, carrying the sidecar's own message. */
export class SidecarError extends Error {
	constructor(message: string) {
		super(message);
		this.name = 'SidecarError';
	}
}

/**
 * Calls one of the shell's sidecar commands and returns its payload.
 *
 * The Tauri `invoke` import is dynamic so this module stays importable in a plain browser,
 * where `@tauri-apps/api` has nothing to talk to.
 *
 * @param command The Tauri command name, e.g. `analyze_replay`.
 * @param args Its arguments.
 * @returns The sidecar's JSON payload.
 * @throws SidecarError When there is no desktop shell, or the sidecar reported a failure. The
 *     message is the sidecar's own where one exists, because it names the replay and the reason
 *     and a generic replacement would throw that away.
 */
async function callSidecar<T>(command: string, args: Record<string, unknown>): Promise<T> {
	const unavailable = sidecarUnavailableReason();
	if (unavailable) throw new SidecarError(unavailable);
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		return await invoke<T>(command, args);
	} catch (error) {
		throw new SidecarError(error instanceof Error ? error.message : String(error));
	}
}

/**
 * Lists every replay file under a directory, without parsing any of them.
 *
 * @param root Directory to search.
 * @param limit Maximum number of files.
 * @returns Replay paths, or an empty list when the shell is unavailable.
 */
export function listReplays(root: string, limit?: number): Promise<string[]> {
	return callSidecar<string[]>('list_replays', { root, limit });
}

/**
 * Parses the headers of every replay under a directory.
 *
 * @param root Directory to search.
 * @param limit Maximum number of replays.
 * @returns One record per replay; a replay the sidecar could not parse carries an `error`
 *     rather than being dropped, so the library can show it as unreadable instead of absent.
 */
export function scanLibrary(root: string, limit?: number): Promise<ReplayHeaderRecord[]> {
	return callSidecar<ReplayHeaderRecord[]>('scan_library', { root, limit });
}

/**
 * Computes every metric for one replay.
 *
 * @param path The replay file.
 * @returns The full analysis payload.
 */
export function analyzeReplay(path: string): Promise<AnalysisPayload> {
	return callSidecar<AnalysisPayload>('analyze_replay', { path });
}

/**
 * Extracts one player's guide-shaped build order.
 *
 * @param path The replay file.
 * @param pid The player, or undefined for all players.
 * @returns Per-pid build orders.
 */
export function replayBuildOrder(
	path: string,
	pid?: number
): Promise<Record<string, { name: string; race: string; steps: BuildOrderStep[] }>> {
	return callSidecar('replay_build_order', { path, pid });
}

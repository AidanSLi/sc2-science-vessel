/**
 * The contract between the Rust/Tauri shell, the Python sidecar, and the UI.
 *
 * This file is the *only* place the shape of a replay-derived value is declared. The
 * frontend never parses a replay (ADR 0001): the sidecar hands over prepared series and
 * summaries shaped like this, and the UI only formats them.
 *
 * Time units are explicit in every name on purpose. The replay clock IS the game clock:
 * from Legacy of the Void onward the SC2 game clock runs 1:1 with real time at `Faster`,
 * so `frames / 16` is both the in-game seconds and the wall-clock seconds of a game with no
 * pauses. The pre-LotV "Blizzard time" offset is gone, and the two must not be conflated
 * anyway — a game clock stops while the game is paused, so it is *not* elapsed real time.
 * See AGENTS.md.
 */

/** The three playable races, as they appear in a replay header. */
export type Race = 'Terran' | 'Protoss' | 'Zerg';

/**
 * A matchup from one player's point of view, e.g. `ZvP`.
 *
 * The first letter is the player's own race. This is the key that selects a preset
 * build order, which is why it is a distinct type rather than a pair of races.
 */
export type Matchup = `${'T' | 'P' | 'Z'}v${'T' | 'P' | 'Z'}`;

export type GameResult = 'Win' | 'Loss' | 'Undecided';

/**
 * A single step of a build order, measured on the in-game clock.
 *
 * `supply` is the supply *cap* the step was made at (6, 10, 14...), which is how every
 * build order is written down and how steps are compared. `timeInGameSeconds` is the
 * clock value the step should occur at, and is what the overlay counts against.
 */
export interface BuildStep {
	/** Supply cap this step was taken at; the canonical build-order key. */
	supply: number;
	/** In-game seconds when this step should happen. */
	timeInGameSeconds: number;
	/** Unit or structure name as the game reports it, e.g. `Drone`, `SpawningPool`. */
	name: string;
	/** True for workers, which build orders often elide. */
	isWorker: boolean;
	/** Whether the step is chronoboosted, for Protoss builds. */
	isChronoboosted: boolean;
}

/** A named build order that can be selected and followed. */
export interface BuildOrder {
	id: string;
	/** Human title, e.g. `22 Drone Ling Bane`. */
	title: string;
	/** Matchup this build is intended for; used to auto-select on game start. */
	matchup: Matchup;
	/** Where the build came from, so the UI can be honest about provenance. */
	source: 'user' | 'imported' | 'bundled';
	/** Upstream attribution when `source` is `imported` or `bundled`. */
	author?: string;
	steps: BuildStep[];
}

/** One player's row in the post-game summary. */
export interface PlayerSummary {
	name: string;
	race: Race;
	team: number;
	result: GameResult;
	/** MMR, when the replay carries it. Only ranked games have it (236 of 722 measured). */
	mmr?: number;
}

/**
 * A game as it appears in the library list, before any metrics are computed.
 *
 * Everything here comes from the replay header, which parses in ~3 ms (measured across
 * 722 replays), so the list can be populated without the expensive event decode.
 */
export interface GameSummary {
	/** Stable id: the replay's path on disk, used as the cache key. */
	id: string;
	path: string;
	mapName: string;
	matchup: Matchup;
	/** Game version string, e.g. `5.0.16.97364`. */
	version: string;
	date: string;
	/** In-game duration in seconds (`frames / 16`), not wall-clock. */
	lengthInGameSeconds: number;
	players: PlayerSummary[];
}

/**
 * A chart series, already reduced to what the chart will draw.
 *
 * The sidecar downsamples before sending: a 25-minute game has tens of thousands of raw
 * tracker events and they must not cross this boundary (ADR 0001).
 */
export interface Series {
	id: string;
	label: string;
	/** Colour token name from app.css, not a raw colour, so themes stay centralised. */
	colorToken: string;
	/** X values in seconds on the in-game clock. */
	x: number[];
	y: number[];
}

/** Which analysis view a set of series belongs to. */
export type MetricGroup = 'economy' | 'military' | 'production' | 'army';

/** A prepared result ready to be rendered by a chart. */
export interface MetricBundle {
	group: MetricGroup;
	series: Series[];
}

/**
 * The SC2 Client API response, as documented on `localhost:6119`.
 *
 * This is the *only* sanctioned live input (ADR 0004). It carries the in-game clock and
 * player identity and nothing else — no units, no supply. Do not extend this type with
 * fields the game does not actually send.
 */
export interface LiveGameState {
	/** True when the client is playing back a replay rather than a live game. */
	isReplay: boolean;
	/**
	 * The UI clock, in seconds. This is the value a build-order timer counts against,
	 * because the game itself freezes it while paused. `null` when the response carried no
	 * numeric clock — which is not the same as `0`, a legitimate reading at game start.
	 */
	displayTime: number | null;
	players: { name: string; type: string; race: Race; result: string }[];
}

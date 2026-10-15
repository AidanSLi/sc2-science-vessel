/**
 * The StarCraft II Client API, polled on loopback.
 *
 * This is the only sanctioned live input the app has (ADR 0004). The client exposes it by
 * default while running, on `localhost:6119`, and it reports exactly two useful things:
 * the **UI clock** (`displayTime`) and who is playing. There is no unit, supply or build
 * data — do not expect any, and do not add code that pretends otherwise.
 *
 * `displayTime` is what makes the build-order guide correct: the game freezes that clock
 * while paused, so a guide driven from it pauses and resumes with the game for free,
 * instead of drifting the way a wall-clock timer would.
 *
 * Two transports are needed. Inside Tauri the request goes through the `http` plugin,
 * because a WebView cannot reach loopback on its own and the capability scope limits us to
 * port 6119. In a plain browser (the frontend is also opened directly in dev) the plugin
 * is unavailable, so this falls back to `fetch` and reports a clear status when CORS
 * blocks it rather than hanging.
 */

import type { LiveGameState, Race } from './types';

/** Port the SC2 client listens on, and the only host/port the app is allowed to reach. */
export const CLIENT_API_PORT = 6119;

const ENDPOINT = `http://localhost:${CLIENT_API_PORT}/game`;

/** How the poller is doing, so the UI can be honest instead of showing a stale clock. */
export type LiveStatus =
	| 'idle'
	| 'connecting'
	| 'live'
	| 'replay'
	| 'unavailable'
	| 'error';

/** The full state of the live connection at one moment. */
export interface LiveState {
	status: LiveStatus;
	/** Parsed client state, when the client answered. */
	game: LiveGameState | null;
	/** Wall-clock milliseconds of the last successful poll. */
	lastOkAt: number | null;
	/** Human-readable reason for `error` or `unavailable`. */
	detail: string | null;
}

/** The raw shape the client returns. Narrowed field-by-field, never trusted wholesale. */
interface RawGameResponse {
	isReplay?: unknown;
	displayTime?: unknown;
	players?: unknown;
}

/**
 * Reports whether the page is running inside a Tauri WebView.
 *
 * @returns `true` when Tauri's IPC bridge is present.
 */
export function isTauri(): boolean {
	return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * Maps a race name as the client reports it onto our own `Race` type.
 *
 * The client does not send the full race name. A **live game at patch 5.0.16 reports
 * `{"race":"Prot"}`** — a four-character abbreviation, not `"Protoss"`. Matching only on full
 * names silently discards that player, and because a player's race is the key that selects a
 * matchup's preset build order (see `MatchupId` in `types.ts`), the opponent would simply
 * vanish from the overlay instead of the app admitting it could not read the race. Zerg is
 * already four characters, so the failure is invisible in a ZvX game and only appears in
 * PvX and TvX — the kind of bug that survives casual testing.
 *
 * Both spellings are accepted: the abbreviation is what the endpoint has historically
 * contained, and the full name is the obvious future change. Anything else returns `null`
 * rather than guessing — `"random"` is genuinely unanswerable until the game resolves it,
 * and a guess would silently load the wrong build order.
 *
 * Deliberately exported: the Python sidecar receives race strings from replay headers and
 * from SpawningTool snapshots too, and all of them must agree on the mapping.
 *
 * @param value The unvalidated race value from a client or sidecar payload.
 * @returns The normalised race, or `null` when it is missing or unrecognised.
 */
export function normaliseRace(value: unknown): Race | null {
	if (typeof value !== 'string') return null;
	switch (value.trim().toLowerCase()) {
		case 'terran':
		case 'terr':
			return 'Terran';
		case 'protoss':
		case 'prot':
			return 'Protoss';
		case 'zerg':
		case 'zergling':
			return 'Zerg';
		default:
			return null;
	}
}

/**
 * Narrows the client's `players` array into our own `Race` type.
 *
 * @param raw The unvalidated `players` value from the response.
 * @returns Normalised players, or an empty list if the shape was unexpected.
 */
function parsePlayers(raw: unknown): LiveGameState['players'] {
	if (!Array.isArray(raw)) return [];
	const out: LiveGameState['players'] = [];
	for (const entry of raw) {
		if (typeof entry !== 'object' || entry === null) continue;
		const p = entry as Record<string, unknown>;
		const normalised = normaliseRace(p.race);
		if (!normalised) continue;
		out.push({
			name: typeof p.name === 'string' ? p.name : 'Unknown',
			type: typeof p.type === 'string' ? p.type : 'user',
			race: normalised,
			result: typeof p.result === 'string' ? p.result : ''
		});
	}
	return out;
}

/**
 * Reads the client API once.
 *
 * Never throws: every failure is reported as a status, because a missing endpoint is an
 * expected state (the game is simply not running) rather than an error the player should
 * see a stack trace for.
 *
 * @returns The polled state.
 */
export async function pollClientApi(): Promise<LiveState> {
	try {
		const response = await request(ENDPOINT);
		if (!response.ok) {
			return {
				status: 'unavailable',
				game: null,
				lastOkAt: null,
				detail: `client API answered HTTP ${response.status}`
			};
		}
		const raw = (await response.json()) as RawGameResponse;
		const game = parseGamePayload(raw);
		if (!game || game.displayTime === null) {
			return {
				status: 'error',
				game: null,
				lastOkAt: null,
				detail: 'client API response had no numeric displayTime'
			};
		}
		return {
			status: game.isReplay ? 'replay' : 'live',
			game,
			lastOkAt: Date.now(),
			detail: null
		};
	} catch (err) {
		const detail = err instanceof Error ? err.message : String(err);
		return { status: 'unavailable', game: null, lastOkAt: null, detail };
	}
}

/**
 * Narrows a raw client payload into a `LiveGameState`.
 *
 * Separated from the fetch so the parsing rules are testable without a network or a running
 * game, and because the payload is the one place the app trusts an external contract. Every
 * field is checked rather than cast: this response comes from a Blizzard binary whose field
 * values have already surprised us once (races arrive as `"Prot"`).
 *
 * `displayTime` stays `null` when absent rather than defaulting to `0`, because zero is a
 * legitimate clock reading at game start — a missing clock must not look like a fresh game,
 * or the overlay would start counting a build order from the beginning.
 *
 * @param raw The parsed JSON body, of unknown shape.
 * @returns The narrowed state, or `null` when the body was not an object at all.
 */
export function parseGamePayload(raw: unknown): LiveGameState | null {
	if (typeof raw !== 'object' || raw === null) return null;
	const body = raw as Record<string, unknown>;
	return {
		isReplay: body.isReplay === true,
		displayTime: typeof body.displayTime === 'number' ? body.displayTime : null,
		players: parsePlayers(body.players)
	};
}

/**
 * Performs the HTTP GET through whichever transport this context supports.
 *
 * @param url The loopback URL to read.
 * @returns The response.
 */
async function request(url: string): Promise<Response> {
	if (isTauri()) {
		const { fetch: tauriFetch } = await import('@tauri-apps/plugin-http');
		return tauriFetch(url);
	}
	// Plain-browser fallback. SC2 sends no CORS headers, so this normally fails in a
	// browser tab — that is what `unavailable` with a network error means, and the UI
	// says so instead of implying the game is closed.
	return fetch(url);
}

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
 * Narrows the client's `players` array into our own `Race` type.
 *
 * The client sends race names in a couple of spellings across versions (`Zerg`,
 * `ZERG`), so this normalises rather than casting — an unrecognised race becomes
 * `Zerg` only if it really says zerg, otherwise it fails loudly.
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
		const race = typeof p.race === 'string' ? p.race.toLowerCase() : '';
		const normalised: Race | null =
			race === 'terran' ? 'Terran' : race === 'protoss' ? 'Protoss' : race === 'zerg' ? 'Zerg' : null;
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
		if (typeof raw.displayTime !== 'number') {
			return {
				status: 'error',
				game: null,
				lastOkAt: null,
				detail: 'client API response had no numeric displayTime'
			};
		}
		const game: LiveGameState = {
			isReplay: raw.isReplay === true,
			displayTime: raw.displayTime,
			players: parsePlayers(raw.players)
		};
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

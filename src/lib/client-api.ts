/**
 * The StarCraft II Client API, polled on loopback.
 *
 * This is the only sanctioned live input the app has (ADR 0004). The client exposes it by
 * default while running, on `localhost:6119`, and it reports three useful things: the **UI
 * clock** (`displayTime`), who is playing, and **which screens are on screen** (the `/ui`
 * endpoint's `activeScreens`). There is no unit, supply or build data — do not expect any,
 * and do not add code that pretends otherwise.
 *
 * `displayTime` is what makes the build-order guide correct: the game freezes that clock
 * while paused, so a guide driven from it pauses and resumes with the game for free,
 * instead of drifting the way a wall-clock timer would.
 *
 * `activeScreens` is what makes the *window mode* correct. It is empty while a game is being
 * played, and contains `ScreenScore/ScreenScore` once the match is over and the player is on
 * the post-game screen — exactly the boundary the app switches views on. This was missed at
 * first: `/ui` was polled once during a game, returned `{"activeScreens":[]}`, and was
 * written off as carrying no information. In fact an empty list *is* the in-game reading,
 * and it is the populated readings that matter. The screen names were recovered from two
 * working consumers of this API (github.com/leigholiver/OBS-SC2Switcher and
 * github.com/Rairden/sc2-scenes, the latter with captured transitions) because there is no
 * official documentation for this endpoint.
 *
 * Two endpoints are therefore read on every poll, and they answer different questions:
 * `/ui` says *where the player is*, `/game` says *what the game is doing*.
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

const GAME_ENDPOINT = `http://localhost:${CLIENT_API_PORT}/game`;
const UI_ENDPOINT = `http://localhost:${CLIENT_API_PORT}/ui`;

/**
 * The client's screen name for the post-game score screen.
 *
 * The list is served as `Screen/<Screen>` pairs, so this is matched as a substring rather
 * than for equality: a captured payload shows `"ScreenScore/ScreenScore"` sitting alongside
 * `ScreenBackgroundSC2`, `ScreenNavigationSC2`, `ScreenForegroundSC2` and `ScreenCustom`.
 */
export const SCORE_SCREEN = 'ScreenScore';

/**
 * The client's screen name shown while a map is loading.
 *
 * A loading screen sits between the lobby and the game, so it is the one reading during which
 * neither view is quite right. It counts as "still a game" in the mode decision, because a
 * player who is loading is about to play and should find the guide already up.
 */
export const LOADING_SCREEN = 'ScreenLoading';

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
	/**
	 * Screen names the client reports as open, or `null` when `/ui` could not be read.
	 *
	 * An empty list is meaningful and is the in-game reading; `null` means unknown. See
	 * `fetchActiveScreens`.
	 */
	screens: string[] | null;
	/** Wall-clock milliseconds of the last successful poll. */
	lastOkAt: number | null;
	/** Human-readable reason for `error` or `unavailable`. */
	detail: string | null;
}

/** The raw shape `/ui` returns. */
interface RawUiResponse {
	activeScreens?: unknown;
}

/**
 * Narrows `/ui`'s `activeScreens` into a list of screen names.
 *
 * Entries the client sends are strings; anything else is dropped rather than stringified,
 * because a screen name is only ever matched as a substring and a coerced non-string could
 * match something by accident.
 *
 * @param raw The unvalidated `activeScreens` value.
 * @returns The screen names, or an empty list if the shape was unexpected.
 */
export function parseActiveScreens(raw: unknown): string[] {
	if (!Array.isArray(raw)) return [];
	return raw.filter((entry): entry is string => typeof entry === 'string');
}

/**
 * Reads which screens the client currently has open.
 *
 * Returns `null` when the request failed, which is deliberately distinct from an empty list:
 * **empty means "no UI screen is open", and that is the reading during a game**, whereas
 * `null` means the app could not tell. Collapsing the two would make a transient failure look
 * exactly like a game, which would put a build-order guide over the player's menus.
 *
 * @returns The active screen names, or `null` when the client could not be reached.
 */
export async function fetchActiveScreens(): Promise<string[] | null> {
	try {
		const response = await request(UI_ENDPOINT);
		if (!response.ok) return null;
		const raw = (await response.json()) as RawUiResponse;
		return parseActiveScreens(raw.activeScreens);
	} catch {
		return null;
	}
}

/**
 * Reports whether a set of active screens contains the given screen.
 *
 * A substring test because the client serves names as `Screen/Screen` pairs, e.g.
 * `"ScreenScore/ScreenScore"`, so the trailing half is redundant and an equality test would
 * have to pick one half to trust.
 *
 * @param screens The active screen names, or `null` when they could not be read.
 * @param name The bare screen name to look for, e.g. `SCORE_SCREEN`.
 * @returns `true` when the screen is among them.
 */
export function isScreenActive(screens: string[] | null, name: string): boolean {
	return screens !== null && screens.some((screen) => screen.includes(name));
}

/** The raw shape `/game` returns. Narrowed field-by-field, never trusted wholesale. */
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
		// Both endpoints are read together, and the screen list is fetched without being
		// allowed to fail the poll: `/ui` is a bonus signal on top of the clock, so a client
		// that answers `/game` but not `/ui` must still produce a usable live state.
		const [response, screens] = await Promise.all([
			request(GAME_ENDPOINT),
			fetchActiveScreens()
		]);
		if (!response.ok) {
			return {
				status: 'unavailable',
				game: null,
				screens,
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
				screens,
				lastOkAt: null,
				detail: 'client API response had no numeric displayTime'
			};
		}
		return {
			status: game.isReplay ? 'replay' : 'live',
			game,
			screens,
			lastOkAt: Date.now(),
			detail: null
		};
	} catch (err) {
		const detail = err instanceof Error ? err.message : String(err);
		return { status: 'unavailable', game: null, screens: null, lastOkAt: null, detail };
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

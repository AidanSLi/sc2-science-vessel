/**
 * Which of its two roles the app's single window is currently playing.
 *
 * The app has **one window with two roles**, not two windows. It is either an ordinary
 * resizable analysis window, or a borderless click-through build-order guide pinned over the
 * game.
 *
 * Two separate things change it, and keeping them straight is the whole point of this module:
 *
 * - The **global hotkey shows and hides the window** and does not touch the mode. It is a
 *   visibility control: press it to bring the app up quickly, and it appears as whichever view
 *   the game state calls for.
 * - **Game state picks the mode**, automatically (see `#lib/game-mode`): a game in progress
 *   means the build guide, and the game ending means the analysis view — including the switch
 *   back the moment the match ends, which is what lets the player leave it up while playing.
 *
 * The shell owns the mode, not the frontend, because the properties being changed
 * (decorations, click-through, size) are not settable from the frontend at all. So this module
 * is a *mirror*: it asks Rust what the mode is and subscribes to changes, and it must never
 * become the source of truth, or the rendered view could disagree with the actual window —
 * an analysis page inside a 380-pixel click-through panel being the worst version of that.
 */
import { writable, get, type Readable } from 'svelte/store';
import { isTauri } from './client-api';

/** The two roles the single window can play. */
export type WindowMode = 'analysis' | 'overlay';

/**
 * Event name the Rust shell emits when the window's mode changes.
 *
 * Must match `WINDOW_MODE_EVENT` in `src-tauri/src/lib.rs`. A mismatch presents as a build
 * guide that never appears on a hotkey press while the window itself has in fact changed
 * shape, which is a confusing bug to diagnose — hence the shared name and this note.
 */
export const WINDOW_MODE_EVENT = 'window-mode';

/** The window's current mode, mirrored from the shell. */
const mode = writable<WindowMode>('analysis');

/** Read-only view of the mode, for components that only render from it. */
export const windowMode: Readable<WindowMode> = { subscribe: mode.subscribe };

/**
 * Narrows an unknown value from Rust or an event payload to a `WindowMode`.
 *
 * Defensive rather than trusting: the value crosses an IPC boundary, and an unrecognised
 * mode must not leave the UI rendering an overlay view inside the ordinary window. Anything
 * unknown resolves to `'analysis'`, the harmless mode — the player can still click things.
 *
 * @param value The raw value to interpret.
 * @returns The matching mode, or `null` if the value is not a mode at all.
 */
export function parseWindowMode(value: unknown): WindowMode | null {
	return value === 'overlay' || value === 'analysis' ? value : null;
}

/**
 * Reads the mode from the shell and subscribes to later changes.
 *
 * Both halves are needed and neither is sufficient: the read covers a mode that changed
 * *before* this ran (the player pressing the hotkey while the frontend was still loading),
 * and the subscription covers every change after. Doing only the read would desynchronise on
 * the first hotkey press; doing only the subscription would miss a startup toggle.
 *
 * In a plain browser there is no shell to ask, so the mode stays `'analysis'` — which is also
 * the right answer, since a browser tab is only ever used for development.
 *
 * @returns A function that removes the listener, suitable for returning from `onMount`.
 */
export async function initWindowMode(): Promise<() => void> {
	if (!isTauri()) return () => {};

	const [{ invoke }, { listen }] = await Promise.all([
		import('@tauri-apps/api/core'),
		import('@tauri-apps/api/event')
	]);

	// Subscribe before reading, not after: a hotkey press landing between the read and the
	// subscribe would otherwise be lost, and the window would sit in the wrong shape until
	// the next press.
	const unlisten = await listen(WINDOW_MODE_EVENT, (event) => {
		const next = parseWindowMode(event.payload);
		if (next) mode.set(next);
	});

	try {
		const current = parseWindowMode(await invoke('get_window_mode'));
		if (current) mode.set(current);
	} catch {
		// The event listener is already live, so a failed initial read only means the mode
		// stays at its default until the next change. Not worth failing startup over.
	}

	return unlisten;
}

/**
 * Asks the shell to switch the window to a mode.
 *
 * Used by the in-app button, which exists so the overlay is reachable without the global
 * hotkey — the hotkey is the compositor's to grant, and on a machine where the portal
 * refuses it the feature would otherwise be unreachable.
 *
 * The store is updated from the *shell's* reply rather than from the argument, so the UI can
 * never claim a mode the window did not actually enter.
 *
 * @param next The mode to switch to.
 * @returns The mode the shell reports the window is now in.
 */
export async function setWindowMode(next: WindowMode): Promise<WindowMode> {
	if (!isTauri()) return get(mode);

	const { invoke } = await import('@tauri-apps/api/core');
	const applied = parseWindowMode(
		await invoke<unknown>('set_window_mode_command', { mode: next })
	);
	if (applied) mode.set(applied);
	return applied ?? get(mode);
}

/**
 * Shows or hides the window, exactly as the global hotkey does.
 *
 * Exists so the in-app control and the global hotkey run the *same* Rust function rather than
 * the frontend deciding visibility for itself — two sources of truth for a window's visibility
 * get out of step the first time the hotkey is used.
 *
 * The mode is left exactly as the shell reports it and is **not** flipped locally: this hides
 * or shows the window, it does not change what the window is.
 *
 * @returns The mode the window remains in (unchanged by this call).
 */
export async function toggleWindow(): Promise<WindowMode> {
	if (!isTauri()) return get(mode);

	const { invoke } = await import('@tauri-apps/api/core');
	await invoke<boolean>('toggle_window');
	return get(mode);
}

/** Resets the mirror to its default, for tests. */
export function resetWindowModeForTest(): void {
	mode.set('analysis');
}

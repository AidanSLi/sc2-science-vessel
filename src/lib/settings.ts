/**
 * Reading and writing the shell's settings over Tauri IPC.
 *
 * Settings live in Rust (they have to: the shortcut is registered with the global-shortcut
 * plugin, and the config directory is a Tauri path). This module is the single place the
 * frontend talks to them, so the "no desktop shell" case — the frontend is also opened
 * directly in a browser during development — is handled once rather than at every call site.
 */

import { isTauri } from './client-api';
import type { Matchup } from './types';

/** The settings the shell persists and exposes. Mirrors `src-tauri/src/settings.rs`. */
export interface AppSettings {
	/** The overlay toggle accelerator that is *actually registered*, e.g. `Ctrl+Alt+O`. */
	overlayShortcut: string;
	/** Directories scanned for `.SC2Replay` files, in the order the user added them. */
	replayRoots: string[];
	/**
	 * Build order id to load per matchup, e.g. `{ ZvP: 'bundled-123' }`.
	 *
	 * A key may be a wildcard (`ZvX`) which is used when no exact matchup entry exists, so a
	 * player with one build per race does not have to triplicate it. Empty means no presets,
	 * which the overlay treats as "use the first build for the matchup".
	 */
	buildPresets: PresetMap;
	/**
	 * Whether the app starts hidden and pops up a hint when a game begins.
	 *
	 * The resident default. When false the app opens with its window visible, the way an
	 * ordinary analysis application would.
	 */
	startHidden: boolean;
	/** Whether the app should be launched when the user logs in. */
	launchAtLogin: boolean;
	/**
	 * The appearance theme id, one of `THEMES` in `#lib/theme`.
	 *
	 * Typed as a plain string, not `ThemeId`, because the value comes from a file the user may
	 * have edited and from a shell that may be a different version; `parseTheme` is what turns
	 * it into a usable id.
	 */
	theme: string;
}

/** Matchup (or `ZvX` wildcard) to build order id. */
export type PresetMap = Partial<Record<Matchup, string>>;

/** The accelerator used when nothing has been chosen, matching the Rust default. */
export const DEFAULT_OVERLAY_SHORTCUT = 'Ctrl+Alt+O';

/** The settings used when the shell is unavailable, matching the Rust defaults. */
const FALLBACK_SETTINGS: AppSettings = {
	overlayShortcut: DEFAULT_OVERLAY_SHORTCUT,
	replayRoots: [],
	buildPresets: {},
	startHidden: true,
	launchAtLogin: false,
	theme: 'terran'
};

/** Outcome of a rebind attempt, so the UI can distinguish success from substitution. */
export interface RebindResult {
	/** The accelerator now registered. */
	shortcut: string;
	/** True when the requested key was taken and a different one was registered instead. */
	substituted: boolean;
}

/**
 * Loads settings from the shell.
 *
 * @returns The settings, or the defaults when running outside Tauri.
 */
export async function loadSettings(): Promise<AppSettings> {
	if (!isTauri()) return { ...FALLBACK_SETTINGS };
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		const settings = await invoke<Partial<AppSettings>>('get_settings');
		// A settings file written by an older build has no `replayRoots`, and a payload with a
		// missing array would make every caller guard against `undefined`. Filling it in here
		// keeps the type honest.
		return {
			overlayShortcut: settings.overlayShortcut ?? DEFAULT_OVERLAY_SHORTCUT,
			replayRoots: settings.replayRoots ?? [],
			buildPresets: settings.buildPresets ?? {},
			// Default-on, so a missing field means true rather than false: a settings file
			// written before this option existed was written when a hidden start was the only
			// behaviour the app had, and reading it as "visible" would change how the app
			// behaves on upgrade.
			startHidden: settings.startHidden ?? true,
			launchAtLogin: settings.launchAtLogin ?? false,
			theme: settings.theme ?? 'terran'
		};
	} catch {
		return { ...FALLBACK_SETTINGS };
	}
}

/**
 * Lists directories that look like replay libraries on this machine.
 *
 * @returns Candidate paths, or an empty list outside the desktop shell or on failure. An
 *     empty list is a normal answer — the game may simply not be installed — so it is not
 *     reported as an error.
 */
export async function suggestReplayRoots(): Promise<string[]> {
	if (!isTauri()) return [];
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		return await invoke<string[]>('suggest_replay_roots');
	} catch {
		return [];
	}
}

/**
 * Replaces the configured replay folders.
 *
 * @param roots The complete list of directories.
 * @returns The stored list, which is the shell's normalised version of the request.
 * @throws When the shell is unavailable or refused to write, with its explanation.
 */
export async function saveReplayRoots(roots: string[]): Promise<string[]> {
	if (!isTauri()) {
		throw new Error('The desktop app is needed to read replay folders from disk.');
	}
	const { invoke } = await import('@tauri-apps/api/core');
	return await invoke<string[]>('set_replay_roots', { roots });
}

/**
 * Reads the configured per-matchup build presets.
 *
 * @returns The preset map, or an empty map outside the desktop shell. Empty is a normal
 *     first-run state, not an error, so callers fall back rather than reporting a problem.
 */
export async function loadPresets(): Promise<{ presets: PresetMap }> {
	const settings = await loadSettings();
	return { presets: settings.buildPresets };
}

/**
 * Stores the per-matchup build presets.
 *
 * The whole map is sent rather than one matchup, so clearing an entry removes it instead of
 * having to distinguish "unset" from "empty" over IPC.
 *
 * @param presets Matchup (or wildcard) to build order id.
 * @returns The stored map, with blank entries dropped by the shell.
 */
export async function savePresets(presets: PresetMap): Promise<PresetMap> {
	if (!isTauri()) {
		throw new Error('The desktop app is needed to remember build presets.');
	}
	const { invoke } = await import('@tauri-apps/api/core');
	return await invoke<PresetMap>('set_build_presets', { presets });
}

/**
 * Rebinds the overlay toggle key.
 *
 * The shell may register a different key than the one requested, because it walks a
 * fallback list when the requested combination is already owned by another application.
 * That substitution is reported rather than hidden: a player whose key silently did not
 * take effect would press it mid-game and see nothing happen.
 *
 * @param shortcut The requested accelerator, e.g. `Ctrl+Alt+O`.
 * @returns The registered accelerator and whether it differs from the request.
 * @throws When no candidate could be registered, with the shell's explanation.
 */
export async function saveOverlayShortcut(shortcut: string): Promise<RebindResult> {
	if (!isTauri()) {
		throw new Error('The desktop app is needed to change the global shortcut.');
	}
	const { invoke } = await import('@tauri-apps/api/core');
	const registered = await invoke<string>('set_overlay_shortcut', { shortcut });
	return { shortcut: registered, substituted: registered !== shortcut };
}

/** Modifier tokens the parser understands, in the order they should be displayed. */
const MODIFIER_ORDER = ['Ctrl', 'Alt', 'Shift', 'Super', 'Cmd', 'Meta'] as const;

/**
 * Builds an accelerator string from a keyboard event.
 *
 * Modifiers are emitted in a fixed order (`Ctrl+Alt+…`) because the shell's parser accepts
 * them in any order but a stable order keeps the stored value readable and comparable.
 * Meta is normalised to `Super`, which is the spelling the accelerator parser recognises on
 * Linux; `Cmd` is accepted too but a keymap written as `Super` works on both Linux and
 * macOS.
 *
 * @param event A keyboard event captured while the user is recording a shortcut.
 * @returns The accelerator, or `null` when the press was only a modifier.
 */
export function acceleratorFromEvent(event: KeyboardEvent): string | null {
	const key = event.key;
	if (['Control', 'Alt', 'Shift', 'Meta', 'AltGraph'].includes(key)) return null;
	if (key === 'Escape') return null;

	const parts: string[] = [];
	if (event.ctrlKey) parts.push('Ctrl');
	if (event.altKey) parts.push('Alt');
	if (event.shiftKey) parts.push('Shift');
	if (event.metaKey) parts.push('Super');

	// Function keys, digits, and letters are named as-is; anything else (arrows, Space,
	// punctuation) is rejected because the accelerator parser would either reject it or
	// resolve it differently than the keycap suggests.
	let named: string | null = null;
	if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) named = key;
	else if (/^[a-zA-Z]$/.test(key)) named = `Key${key.toUpperCase()}`;
	else if (/^[0-9]$/.test(key)) named = `Digit${key}`;

	if (!named) return null;
	// A bare key with no modifier would steal that key from the game globally, which is
	// never what the player wants.
	if (parts.length === 0) return null;

	// The parser understands both `KeyO` and plain `O`; plain is what a human expects to
	// read on the settings page, so strip the prefix for display and storage.
	const display = named.startsWith('Key')
		? named.slice(3)
		: named.startsWith('Digit')
			? named.slice(5)
			: named;
	return [...parts, display].join('+');
}

/**
 * Reorders an accelerator's modifiers into the canonical display order.
 *
 * @param accelerator An accelerator such as `Alt+Ctrl+O`.
 * @returns The same accelerator with modifiers in {@link MODIFIER_ORDER}.
 */
export function normaliseAccelerator(accelerator: string): string {
	const tokens = accelerator.split('+').map((t) => t.trim()).filter(Boolean);
	const mods = tokens.filter((t) =>
		(MODIFIER_ORDER as readonly string[]).includes(t),
	);
	const keys = tokens.filter((t) => !(MODIFIER_ORDER as readonly string[]).includes(t));
	mods.sort(
		(a, b) =>
			(MODIFIER_ORDER as readonly string[]).indexOf(a) -
			(MODIFIER_ORDER as readonly string[]).indexOf(b),
	);
	return [...mods, ...keys].join('+');
}

/**
 * Turns the resident start on or off.
 *
 * Enabling it hides the window immediately in the shell, because a setting that says the app
 * is resident while its window sits on screen would be describing something that is not true.
 * Disabling it only affects the next launch — the player is looking at the window, and hiding
 * it in response to a switch they turned off would be perverse.
 *
 * @param enabled Whether the app should start hidden.
 * @returns The stored value.
 * @throws When the shell is unavailable or refused to write.
 */
export async function saveStartHidden(enabled: boolean): Promise<boolean> {
	if (!isTauri()) {
		throw new Error('The desktop app is needed to change how it starts.');
	}
	const { invoke } = await import('@tauri-apps/api/core');
	return await invoke<boolean>('set_start_hidden', { enabled });
}

/**
 * Turns "launch at login" on or off, writing the desktop's own autostart entry.
 *
 * @param enabled Whether to launch the app at login.
 * @returns The stored value.
 * @throws When the shell is unavailable, or when the running build is not an installed one —
 *     an autostart entry pointing at a development binary would break on the next rebuild, so
 *     the shell refuses rather than writing one.
 */
export async function saveLaunchAtLogin(enabled: boolean): Promise<boolean> {
	if (!isTauri()) {
		throw new Error('The desktop app is needed to change how it starts.');
	}
	const { invoke } = await import('@tauri-apps/api/core');
	return await invoke<boolean>('set_launch_at_login', { enabled });
}

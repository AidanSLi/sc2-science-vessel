/**
 * Reading and writing the shell's settings over Tauri IPC.
 *
 * Settings live in Rust (they have to: the shortcut is registered with the global-shortcut
 * plugin, and the config directory is a Tauri path). This module is the single place the
 * frontend talks to them, so the "no desktop shell" case — the frontend is also opened
 * directly in a browser during development — is handled once rather than at every call site.
 */

import { isTauri } from './client-api';

/** The settings the shell persists and exposes. Mirrors `src-tauri/src/settings.rs`. */
export interface AppSettings {
	/** The overlay toggle accelerator that is *actually registered*, e.g. `Ctrl+Alt+O`. */
	overlayShortcut: string;
}

/** The accelerator used when nothing has been chosen, matching the Rust default. */
export const DEFAULT_OVERLAY_SHORTCUT = 'Ctrl+Alt+O';

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
	if (!isTauri()) return { overlayShortcut: DEFAULT_OVERLAY_SHORTCUT };
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		return await invoke<AppSettings>('get_settings');
	} catch {
		return { overlayShortcut: DEFAULT_OVERLAY_SHORTCUT };
	}
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

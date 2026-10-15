/**
 * Unit tests for the accelerator plumbing behind the overlay toggle setting.
 *
 * These are pure functions, so they need no Tauri shell. That matters: the settings page's
 * recorder is the only UI in the app that listens for a keypress, and a mistake there is
 * invisible in review — the bug surfaces as "the key I chose does nothing" mid-game, long
 * after the code was written.
 */

import { describe, expect, it } from 'vitest';

import { acceleratorFromEvent, normaliseAccelerator } from './settings';

/**
 * Builds the subset of a `KeyboardEvent` that `acceleratorFromEvent` reads.
 *
 * @param key The `event.key` value.
 * @param modifiers Which modifier keys are held.
 * @returns A stand-in for a real KeyboardEvent.
 */
function press(
	key: string,
	modifiers: { ctrl?: boolean; alt?: boolean; shift?: boolean; meta?: boolean } = {},
): KeyboardEvent {
	return {
		key,
		ctrlKey: modifiers.ctrl ?? false,
		altKey: modifiers.alt ?? false,
		shiftKey: modifiers.shift ?? false,
		metaKey: modifiers.meta ?? false,
	} as KeyboardEvent;
}

describe('acceleratorFromEvent', () => {
	it('builds a chord from a modifier and a letter', () => {
		expect(acceleratorFromEvent(press('o', { ctrl: true, alt: true }))).toBe('Ctrl+Alt+O');
	});

	it('emits modifiers in a stable order regardless of the order held', () => {
		const a = acceleratorFromEvent(press('o', { ctrl: true, alt: true, shift: true }));
		const b = acceleratorFromEvent(press('o', { shift: true, alt: true, ctrl: true }));
		expect(a).toBe('Ctrl+Alt+Shift+O');
		expect(b).toBe(a);
	});

	it('normalises Meta to Super, the spelling the parser recognises', () => {
		expect(acceleratorFromEvent(press('9', { meta: true, alt: true }))).toBe('Alt+Super+9');
	});

	it('accepts function keys', () => {
		expect(acceleratorFromEvent(press('F9', { ctrl: true, alt: true }))).toBe('Ctrl+Alt+F9');
	});

	it('ignores a lone modifier press', () => {
		// Recording starts on a modifier press if the user reaches for Ctrl first; treating
		// that as the shortcut would bind the key to the modifier itself.
		expect(acceleratorFromEvent(press('Control', { ctrl: true }))).toBeNull();
		expect(acceleratorFromEvent(press('Alt', { alt: true }))).toBeNull();
		expect(acceleratorFromEvent(press('Shift', { shift: true }))).toBeNull();
		expect(acceleratorFromEvent(press('Meta', { meta: true }))).toBeNull();
	});

	it('refuses a bare key with no modifier', () => {
		// A global grab on a bare key would steal it from the game.
		expect(acceleratorFromEvent(press('o'))).toBeNull();
	});

	it('refuses keys whose accelerator spelling would not round-trip', () => {
		expect(acceleratorFromEvent(press(' ', { ctrl: true }))).toBeNull();
		expect(acceleratorFromEvent(press('ArrowLeft', { ctrl: true, alt: true }))).toBeNull();
		expect(acceleratorFromEvent(press(';', { ctrl: true, alt: true }))).toBeNull();
		expect(acceleratorFromEvent(press('Escape', { ctrl: true }))).toBeNull();
	});
});

describe('normaliseAccelerator', () => {
	it('reorders modifiers into Ctrl, Alt, Shift, Super', () => {
		expect(normaliseAccelerator('Shift+Ctrl+O')).toBe('Ctrl+Shift+O');
		expect(normaliseAccelerator('Super+Alt+Ctrl+K')).toBe('Ctrl+Alt+Super+K');
	});

	it('leaves an already-canonical accelerator alone', () => {
		expect(normaliseAccelerator('Ctrl+Alt+O')).toBe('Ctrl+Alt+O');
	});

	it('tolerates stray whitespace', () => {
		expect(normaliseAccelerator(' Ctrl + Alt + O ')).toBe('Ctrl+Alt+O');
	});
});

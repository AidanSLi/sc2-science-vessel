/**
 * Tests for the theme module.
 *
 * Two things are worth pinning here, and neither is cosmetic.
 *
 * The first is *validation*. The theme id reaches the document from two directions that the code
 * does not control — a `settings.json` the user can hand-edit, and an IPC reply — and an
 * unrecognised id written into `data-theme` silently unstyles the app, because every
 * `html[data-theme="…"]` block stops matching while `@theme`'s defaults are also overridden away
 * from the element. It fails without an error, on the next launch, far from the cause.
 *
 * The second is the *change notification*. `applyTheme` has to fire it, because the sky layers are
 * markup and cannot see a CSS custom-property change: without the event, picking Space in Settings
 * repaints the palette but leaves no stars until the app is restarted.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';

import { applyTheme, DEFAULT_THEME, onThemeChange, parseTheme, themeHasSky, THEMES } from './theme';

describe('parseTheme', () => {
	it('accepts every offered theme', () => {
		for (const theme of THEMES) expect(parseTheme(theme.id)).toBe(theme.id);
	});

	it('falls back to the default for an unknown or missing id', () => {
		// An id from a newer build, a typo in a hand-edited settings file, and the first-run case
		// where the field is simply absent. All three must land on a complete palette.
		expect(parseTheme('protoss-2')).toBe(DEFAULT_THEME);
		expect(parseTheme('ZERG')).toBe(DEFAULT_THEME);
		expect(parseTheme(undefined)).toBe(DEFAULT_THEME);
		expect(parseTheme(null)).toBe(DEFAULT_THEME);
		expect(parseTheme(42)).toBe(DEFAULT_THEME);
	});
});

describe('themeHasSky', () => {
	it('is true only for Space', () => {
		// The ambient layers are markup, so a theme that reported true here without the CSS to go
		// with it would mount two empty fixed divs over every page.
		expect(themeHasSky('space')).toBe(true);
		expect(themeHasSky('terran')).toBe(false);
		expect(themeHasSky('zerg')).toBe(false);
		expect(themeHasSky('protoss')).toBe(false);
	});
});

describe('applyTheme', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		delete (globalThis as { document?: unknown }).document;
	});

	/** Installs a minimal stand-in for the bits of `document` the module touches. */
	function stubDocument() {
		const attributes: Record<string, string> = {};
		const listeners = new Map<string, Set<(event: Event) => void>>();
		const document = {
			documentElement: {
				dataset: attributes
			},
			addEventListener(type: string, handler: (event: Event) => void) {
				const set = listeners.get(type) ?? new Set();
				set.add(handler);
				listeners.set(type, set);
			},
			removeEventListener(type: string, handler: (event: Event) => void) {
				listeners.get(type)?.delete(handler);
			},
			dispatchEvent(event: Event) {
				for (const handler of listeners.get(event.type) ?? []) handler(event);
				return true;
			}
		};
		vi.stubGlobal('document', document);
		return { attributes, document };
	}

	it('writes the theme onto the root element', () => {
		const { attributes } = stubDocument();
		applyTheme('zerg');
		expect(attributes.theme).toBe('zerg');
	});

	it('announces a change so markup can react', () => {
		stubDocument();
		const seen: string[] = [];
		const stop = onThemeChange((theme) => seen.push(theme));

		applyTheme('protoss');
		applyTheme('space');

		expect(seen).toEqual(['protoss', 'space']);
		stop();
		// A listener removed on teardown must not keep receiving: the layout re-mounts on every
		// navigation, so a leaked one would accumulate handlers.
		applyTheme('terran');
		expect(seen).toEqual(['protoss', 'space']);
	});

	it('stays silent when the theme is already applied', () => {
		// `initTheme` applies the stored theme on every mount, including one that lands on the
		// value `app.html` already wrote. Firing the event then would remount the sky layer on
		// every navigation for no reason.
		stubDocument();
		let count = 0;
		onThemeChange(() => count++);

		applyTheme('terran');
		applyTheme('terran');

		expect(count).toBe(1);
	});

	it('treats the event payload as untrusted', () => {
		stubDocument();
		const seen: string[] = [];
		onThemeChange((theme) => seen.push(theme));

		// The payload only ever comes from this module today, but it crosses a `CustomEvent`
		// boundary, which anything on the page can dispatch on. Parsing it keeps a hand-written
		// event from putting an unknown id into component state.
		document.dispatchEvent(
			new CustomEvent('sc2sv:theme-change', { detail: 'not-a-theme' })
		);

		expect(seen).toEqual([DEFAULT_THEME]);
	});
});

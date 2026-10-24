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

import { readFileSync } from 'node:fs';

import {
	applyTheme,
	DEFAULT_THEME,
	initAnimations,
	onThemeChange,
	parseTheme,
	themeHasSky,
	THEMES
} from './theme';

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

	it('writes the motion capability alongside the theme', () => {
		// These two attributes are set in one place on purpose: a theme applied without a matching
		// `data-motion` is the bug this shape prevents — a static Space background on a platform
		// that could animate it, or an animating one on a platform where that costs a CPU core
		// (ADR 0021). Outside the desktop shell the answer is the safe one, so a developer
		// running `pnpm dev` on Linux sees the shipping behaviour.
		const { attributes } = stubDocument();
		applyTheme('space');
		expect(attributes.motion).toBe('off');
	});

	it('defaults motion off and reports it', async () => {
		// The whole point of the flag is that the *first* themed frame is never the expensive one,
		// so an unanswered query must leave animation disabled rather than enabled.
		stubDocument();
		await expect(initAnimations()).resolves.toBe(false);
	});

	it('gives every theme two distinct colours', () => {
		// The palettes are CSS, so this reads the stylesheet rather than importing it — but the
		// invariant it checks is the one the whole two-tone design rests on, and it broke before:
		// Terran's secondary was 0.038 chroma, which is visually indistinguishable from the
		// surface ramp it sat on, so the theme read as a single-colour theme even though it
		// technically declared two. A test is the only thing that will notice that regressing.
		// Read from disk rather than imported: a `?raw` import of `.css` returns an empty module
		// here, because the Tailwind Vite plugin handles `.css` requests itself. See
		// `src/node-shims.d.ts`.
		const css = readFileSync(new URL('../app.css', import.meta.url), 'utf8');

		for (const theme of THEMES) {
			const block = css.split(`html[data-theme='${theme.id}']`)[1];
			expect(block, `no block for ${theme.id}`).toBeTruthy();
			const scoped = block.slice(0, block.indexOf('}'));
			const accent = /--color-accent:\s*oklch\(([\d.]+) ([\d.]+) (\d+)\)/.exec(scoped);
			const secondary = /--color-accent-2:\s*oklch\(([\d.]+) ([\d.]+) (\d+)\)/.exec(
				scoped
			);
			expect(accent, `no accent for ${theme.id}`).toBeTruthy();
			expect(secondary, `no accent-2 for ${theme.id}`).toBeTruthy();

			// Different hues, not just different lightness: two greys are still one colour to the
			// eye, and the secondary is supposed to be the *second* colour.
			const hueGap = Math.abs(Number(accent![3]) - Number(secondary![3]));
			expect(hueGap, `${theme.id} hues are too close`).toBeGreaterThan(40);

			// Chroma is the "is this colour at all" measure. 0.06 is the floor at which the hue
			// survives a screenshot, which is how the Terran palette was caught.
			expect(Number(secondary![2]), `${theme.id} secondary is too grey`).toBeGreaterThanOrEqual(
				0.06
			);
		}
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

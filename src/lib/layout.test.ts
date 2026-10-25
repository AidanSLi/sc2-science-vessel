import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/**
 * Guards the analysis layout's one structural invariant: the title bar sits OUTSIDE the scrolling
 * area, as its flex sibling above it.
 *
 * The bug this prevents is geometric and was reported as "the scroll bar takes up the entire
 * height". While `.app-surface` was itself the scroll container, the bar had to be `sticky top-0`
 * to survive scrolling — but the scrollbar belonged to the container, so it still ran the window's
 * full height and drew over the bar's right end. Measured on the real window: the 6px scrollbar
 * began at y=0, level with the bar's top edge instead of at the bar's bottom rule.
 *
 * A unit test cannot lay out CSS, so the invariant is asserted on the source that defines it:
 * `.app-surface` is a flex column with no scrolling of its own, `.app-scroll` is the only thing
 * that scrolls, and the root layout renders the bar as a sibling BEFORE `.app-scroll`.
 */
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');

/**
 * Collapses a CSS rule body to `prop: value` pairs with whitespace normalised.
 *
 * Comments are stripped FIRST, and that is load-bearing rather than tidiness: the rule bodies in
 * app.css are half prose, and a comment mentioning `overflow-y: auto` (which the comment on
 * `.app-surface` does, to explain the bug it describes) would otherwise be parsed as a real
 * declaration and make the assertion read the prose instead of the code. Turning comments into a
 * lone `;` also stops two declarations either side of a comment from being joined into one.
 *
 * @param body The text between a rule's braces.
 * @returns Every declaration in it, keyed by property.
 */
function declarations(body: string): Record<string, string> {
	const out: Record<string, string> = {};
	for (const line of body.replace(/\/\*[\s\S]*?\*\//g, ';').split(';')) {
		const [prop, ...rest] = line.split(':');
		if (!prop || rest.length === 0) continue;
		out[prop.trim()] = rest.join(':').trim();
	}
	return out;
}

/** Extracts one rule's body by its selector, returning '' when it is absent. */
function rule(css: string, selector: string): string {
	const at = css.indexOf(`${selector} {`);
	if (at === -1) return '';
	const open = css.indexOf('{', at);
	const close = css.indexOf('}', open);
	return close === -1 ? '' : css.slice(open + 1, close);
}

describe('the analysis layout', () => {
	it('makes the surface a flex column that does not scroll itself', () => {
		const surface = declarations(rule(read('../app.css'), '.app-surface'));
		// The scroll container is the child, not the surface. `overflow-y: auto` here is the bug:
		// it puts the scrollbar on the element that also contains the title bar.
		expect(surface['overflow-y']).not.toBe('auto');
		expect(surface['overflow-y']).not.toBe('scroll');
		expect(surface['display']).toBe('flex');
		expect(surface['flex-direction']).toBe('column');
	});

	it('gives the scrolling child the overflow and a height it may shrink from', () => {
		const scroll = declarations(rule(read('../app.css'), '.app-scroll'));
		expect(scroll['overflow-y']).toBe('auto');
		// `min-height: 0` is load-bearing in a flex column: the default `min-height: auto` refuses
		// to shrink below the content, so the overflow escapes to the window instead of scrolling.
		expect(scroll['min-height']).toBe('0');
		expect(scroll['flex']).toContain('1');
	});

	it('renders the title bar outside the scroll container, above it', () => {
		const layout = read('../routes/+layout.svelte');
		const bar = layout.indexOf('<TitleBar />');
		const scroll = layout.indexOf('class="app-scroll"');
		expect(bar, 'the root layout must render <TitleBar />').toBeGreaterThan(-1);
		expect(scroll, 'the root layout must render .app-scroll').toBeGreaterThan(-1);
		// Order is what makes them siblings rather than parent and child: if the bar were inside
		// `.app-scroll` it would be a descendant and would scroll with the page.
		expect(bar).toBeLessThan(scroll);
	});

	it('does not nest the title bar inside the page body any more', () => {
		// The `(app)` layout is the page body. A bar rendered there is inside `.app-scroll` by
		// construction, which is exactly the arrangement that produced the full-height scrollbar.
		const appLayout = read('../routes/(app)/+layout.svelte');
		expect(appLayout).not.toContain('<TitleBar />');
	});

	it('keeps the bar a plain flex item rather than a sticky one', () => {
		// `sticky` was the workaround for the bar living inside the scroll container. Now that it
		// is structurally outside, `sticky` would be a second, redundant mechanism whose presence
		// would suggest the old arrangement is still supported.
		const bar = read('../lib/components/TitleBar.svelte');
		expect(bar).not.toContain('sticky top-0');
		expect(bar).toContain('shrink-0');
	});
});

/**
 * Guards the frameless window's resize border against the failure that shipped.
 *
 * The bug was not a typo, it was a category error: a handle is a rectangle somewhere in the page,
 * while `startResizeDragging` names a side of the WINDOW. The eight handles were absolutely
 * positioned inside the 36px title bar, so the one labelled `South` lay across the bar's bottom
 * edge and resizing dragged the window's bottom — the user saw a strip under the title bar that
 * resized the window. `North`, `West` and `East` happened to coincide with the window edges, which
 * is why only one edge looked wrong.
 *
 * Nothing in a browser makes that visible: the CSS is valid, the directions are valid, and each
 * handle still resizes *something*. So the invariants are asserted here instead, against the same
 * numbers the stylesheet uses.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { CORNER_SIZE, EDGE_THICKNESS, windowResizeEdges } from './window-edges';

/** A rectangle in CSS pixels, in viewport coordinates. */
interface Box {
	left: number;
	top: number;
	right: number;
	bottom: number;
}

/** The viewport the geometry is solved in. An odd height catches rounding assumptions. */
const VIEWPORT = { width: 1280, height: 821 };

/**
 * Solves a handle's rectangle from its placement name.
 *
 * Mirrors the stylesheet rule for rule. `the_stylesheet_carries_the_numbers_solved_here` is what
 * keeps the two from drifting apart, since this file cannot lay out CSS.
 *
 * Every branch sets BOTH axes of the box, so a wrong sign shows up as a negative width rather than
 * being silently inherited from a neighbouring case. (The first version of this function mixed
 * `left`/`right` in the north-east and south-east cases, and the resulting "handle" was 1248px
 * wide and extended off the viewport — which the overlap check caught immediately.)
 *
 * @param placement The placement key from `windowResizeEdges`.
 * @returns The box, in viewport coordinates.
 */
function solve(placement: string): Box {
	const { width, height } = VIEWPORT;
	switch (placement) {
		case 'north':
			return { left: CORNER_SIZE, right: width - CORNER_SIZE, top: 0, bottom: EDGE_THICKNESS };
		case 'south':
			return {
				left: CORNER_SIZE,
				right: width - CORNER_SIZE,
				top: height - EDGE_THICKNESS,
				bottom: height
			};
		case 'west':
			return { left: 0, right: EDGE_THICKNESS, top: CORNER_SIZE, bottom: height - CORNER_SIZE };
		case 'east':
			return {
				left: width - EDGE_THICKNESS,
				right: width,
				top: CORNER_SIZE,
				bottom: height - CORNER_SIZE
			};
		case 'north-west':
			return { left: 0, right: CORNER_SIZE, top: 0, bottom: CORNER_SIZE };
		case 'north-east':
			return { left: width - CORNER_SIZE, right: width, top: 0, bottom: CORNER_SIZE };
		case 'south-west':
			return { left: 0, right: CORNER_SIZE, top: height - CORNER_SIZE, bottom: height };
		case 'south-east':
			return { left: width - CORNER_SIZE, right: width, top: height - CORNER_SIZE, bottom: height };
		default:
			throw new Error(`unknown placement ${placement}`);
	}
}

describe('the frameless window resize border', () => {
	it('has exactly eight handles, one per resize direction', () => {
		expect(windowResizeEdges).toHaveLength(8);
		expect(new Set(windowResizeEdges.map((edge) => edge.direction)).size).toBe(8);
		expect(new Set(windowResizeEdges.map((edge) => edge.placement)).size).toBe(8);
	});

	it('names the direction that matches the edge the handle sits on', () => {
		// The specific inversion that shipped: the south handle must be at the window's bottom.
		const south = windowResizeEdges.find((edge) => edge.direction === 'South');
		expect(south?.placement).toBe('south');
		expect(solve('south').bottom).toBe(VIEWPORT.height);
		expect(solve('south').top).toBe(VIEWPORT.height - EDGE_THICKNESS);
	});

	it('puts no two handles in the same place', () => {
		// Overlapping handles make the element under the pointer name an arbitrary direction, which
		// is the same class of bug as the one this file exists for: the pointer and the resized edge
		// disagree.
		const boxes = windowResizeEdges.map((edge) => ({ name: edge.placement, box: solve(edge.placement) }));
		for (const [index, first] of boxes.entries()) {
			for (const second of boxes.slice(index + 1)) {
				const overlaps =
					first.box.left < second.box.right &&
					second.box.left < first.box.right &&
					first.box.top < second.box.bottom &&
					second.box.top < first.box.bottom;
				expect(overlaps, `${first.name} overlaps ${second.name}`).toBe(false);
			}
		}
	});

	it('keeps every handle inside the viewport with a positive area', () => {
		// A handle with a negative width is not a smaller handle, it is no handle at all — and it is
		// what a mixed-up left/right pair produces, so the failure is worth naming.
		for (const edge of windowResizeEdges) {
			const box = solve(edge.placement);
			expect(box.right - box.left, `${edge.placement} width`).toBeGreaterThan(0);
			expect(box.bottom - box.top, `${edge.placement} height`).toBeGreaterThan(0);
			expect(box.left, `${edge.placement} left`).toBeGreaterThanOrEqual(0);
			expect(box.top, `${edge.placement} top`).toBeGreaterThanOrEqual(0);
			expect(box.right, `${edge.placement} right`).toBeLessThanOrEqual(VIEWPORT.width);
			expect(box.bottom, `${edge.placement} bottom`).toBeLessThanOrEqual(VIEWPORT.height);
		}
	});

	it('never leaves a gap where the pointer would resize nothing', () => {
		// Every point along each window boundary must belong to some handle. The rare gaps that matter
		// are at the joins, which is exactly where an inset that is not mirrored produces one.
		const boxes = windowResizeEdges.map((edge) => solve(edge.placement));
		const covered = (x: number, y: number) =>
			boxes.some((box) => x >= box.left && x < box.right && y >= box.top && y < box.bottom);
		for (let x = 0; x < VIEWPORT.width; x += 1) {
			expect(covered(x, 0), `top boundary at x=${x}`).toBe(true);
			expect(covered(x, VIEWPORT.height - 1), `bottom boundary at x=${x}`).toBe(true);
		}
		for (let y = 0; y < VIEWPORT.height; y += 1) {
			expect(covered(0, y), `left boundary at y=${y}`).toBe(true);
			expect(covered(VIEWPORT.width - 1, y), `right boundary at y=${y}`).toBe(true);
		}
	});

	it('the stylesheet carries the numbers solved here', () => {
		// A unit test cannot lay out CSS, so the invariant is checked the other way round: the rules
		// must contain the exact insets this file solves with, and the component must carry the
		// matching placement names. A change to one without the other fails here rather than in a
		// screenshot nobody takes again.
		const css = readFileSync(new URL('../app.css', import.meta.url), 'utf8');
		const component = readFileSync(
			new URL('./components/WindowEdges.svelte', import.meta.url),
			'utf8'
		);

		for (const edge of windowResizeEdges) {
			expect(css, `app.css has no rule for ${edge.placement}`).toContain(
				`[data-placement='${edge.placement}']`
			);
		}
		expect(css).toContain(`left: ${CORNER_SIZE / 16}rem`);
		expect(css).toContain(`height: ${EDGE_THICKNESS}px`);
		expect(css).toContain(`width: ${EDGE_THICKNESS}px`);
		expect(css).toContain(`width: ${CORNER_SIZE}px`);
		expect(css).toContain('position: fixed');
		expect(component).toContain('data-placement={edge.placement}');
		expect(component).toContain('window-resize-edge');

		// The title bar must no longer carry resize handles at all. It is `sticky`, so anything
		// positioned inside it is measured against the bar rather than against the window — which is
		// precisely how a `South` handle ended up resizing the window from under the title bar.
		const titleBar = readFileSync(new URL('./components/TitleBar.svelte', import.meta.url), 'utf8');
		// The doc comment in TitleBar.svelte mentions `startResizeDragging` on purpose (it records this
		// very bug), so the assertion is about CALLS, not about the word.
		expect(titleBar).not.toMatch(/void startResize\(/);
		expect(titleBar).not.toContain('window-resize-edge');
	});
});

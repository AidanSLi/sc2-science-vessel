/**
 * The frameless window's resize border, as data rather than as markup buried in one component.
 *
 * The window is declared `decorations: false`, so the toolkit draws no resize border and the app
 * has to provide one. Two facts make this easy to get wrong, and both are costly:
 *
 * 1. **A handle's box and the edge it resizes are two different things.** `startResizeDragging`
 *    names a SIDE OF THE WINDOW; a handle is a rectangle somewhere in the page. If that rectangle
 *    is positioned relative to anything smaller than the window, the two disagree. That is exactly
 *    what shipped: the eight handles were absolutely positioned inside the 36px-tall title bar, so
 *    `bottom-0` resolved against the *bar's* bottom edge while `'South'` resized the *window's*
 *    bottom — dragging the bottom of the title bar resized the bottom of the window. The
 *    `North`/`West`/`East` handles happened to line up with the window's own edges, which is why
 *    only one edge looked broken.
 *
 * 2. **A window edge is not part of the page.** The title bar is chrome that lives above the
 *    scrolling body (ADR 0027), and the window's edges are further out still: they never move, and
 *    they are not content. So a handle cannot be drawn inside the bar however it is positioned.
 *
 * Hence: handles live in `WindowEdges.svelte`, anchored to the viewport by `position: fixed` (see
 * `.window-resize-edge` in app.css, which for a frameless window IS the window).
 *
 * The geometry is deliberately non-overlapping rather than merely non-gappy. Overlapping handles
 * would make the element under the pointer name an arbitrary direction — the same class of bug as
 * the one above, where the pointer and the resized edge disagree. `src/lib/window-edges.test.ts`
 * solves the rectangles and asserts they never intersect.
 */

/** One resize handle: the direction to hand the window manager, a cursor, and where it sits. */
export interface ResizeEdge {
	/** One of Tauri's `ResizeDirection` variants. */
	direction:
		| 'North'
		| 'South'
		| 'West'
		| 'East'
		| 'NorthWest'
		| 'NorthEast'
		| 'SouthWest'
		| 'SouthEast';
	/** The CSS cursor, matching what the direction does. */
	cursor: string;
	/**
	 * Which corner or edge this handle occupies.
	 *
	 * Expressed as a key rather than as raw classes so the geometry lives in `app.css` next to the
	 * comments explaining it, and so a test can solve every rectangle and prove they stay apart.
	 */
	placement:
		| 'north'
		| 'south'
		| 'west'
		| 'east'
		| 'north-west'
		| 'north-east'
		| 'south-west'
		| 'south-east';
}

/** The eight resize handles. Order is cosmetic; the boxes never overlap, by construction. */
export const windowResizeEdges: readonly ResizeEdge[] = [
	{ direction: 'North', cursor: 'cursor-ns-resize', placement: 'north' },
	{ direction: 'South', cursor: 'cursor-ns-resize', placement: 'south' },
	{ direction: 'West', cursor: 'cursor-ew-resize', placement: 'west' },
	{ direction: 'East', cursor: 'cursor-ew-resize', placement: 'east' },
	{ direction: 'NorthWest', cursor: 'cursor-nwse-resize', placement: 'north-west' },
	{ direction: 'NorthEast', cursor: 'cursor-nesw-resize', placement: 'north-east' },
	{ direction: 'SouthWest', cursor: 'cursor-nesw-resize', placement: 'south-west' },
	{ direction: 'SouthEast', cursor: 'cursor-nwse-resize', placement: 'south-east' }
] as const;

/**
 * Thickness of an edge handle, in CSS pixels.
 *
 * Thin enough to be invisible over the page, thick enough to hit without aiming — the pointer
 * stops at a window edge, so the effective target is larger than this on the outside.
 */
export const EDGE_THICKNESS = 4;

/**
 * Side of a corner handle, in CSS pixels.
 *
 * The edges are inset by exactly this much, so a corner and an edge never share a pixel. A corner
 * that merely overlapped an edge would still work, but which direction you got would depend on DOM
 * order, and that is the kind of rule that breaks silently when a list is reordered.
 */
export const CORNER_SIZE = 16;

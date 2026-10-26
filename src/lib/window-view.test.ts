import { describe, expect, it } from 'vitest';
import { OVERLAY_WINDOW_LABEL, parseWindowView, viewForLabel } from './window-view';

describe('parseWindowView', () => {
	it('accepts the two view names', () => {
		expect(parseWindowView('analysis')).toBe('analysis');
		expect(parseWindowView('overlay')).toBe('overlay');
	});

	it('rejects anything else, so the caller has to choose a default', () => {
		expect(parseWindowView('overlays')).toBeNull();
		expect(parseWindowView('')).toBeNull();
		expect(parseWindowView(undefined)).toBeNull();
		expect(parseWindowView(null)).toBeNull();
		expect(parseWindowView(7)).toBeNull();
	});
});

describe('viewForLabel', () => {
	it('gives the overlay window the guide', () => {
		expect(viewForLabel(OVERLAY_WINDOW_LABEL)).toBe('overlay');
	});

	it('gives every other label the analysis view', () => {
		// `main` is the real analysis window; the empty string is a plain browser tab, which has
		// no label at all. Both must render the usable view rather than a click-through panel.
		expect(viewForLabel('main')).toBe('analysis');
		expect(viewForLabel('')).toBe('analysis');
		expect(viewForLabel(null)).toBe('analysis');
		expect(viewForLabel(undefined)).toBe('analysis');
	});

	it('does not treat a lookalike label as the overlay', () => {
		// The label is compared exactly: a near-miss must not claim the guide's view, because a
		// text-only transparent panel elsewhere would be unreadable and unclickable.
		expect(viewForLabel('Overlay')).toBe('analysis');
		expect(viewForLabel('overlay ')).toBe('analysis');
	});
});

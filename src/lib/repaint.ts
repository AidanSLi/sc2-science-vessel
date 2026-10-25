/**
 * Keeps a transparent WebKitGTK view's backing store alive across repaints.
 *
 * The app's window is declared `transparent: true`, and `wry` 0.57.0 applies that to the web
 * view at creation (`src/webkitgtk/mod.rs:293-295`). Under the software renderer this app is
 * forced onto (`WEBKIT_DISABLE_DMABUF_RENDERER=1`, ADR 0005 — the GPU path fails with
 * "Failed to create GBM buffer"), a view that was transparent *from creation* discards its
 * backing store whenever something repaints and redraws only the damaged rectangle. The rest of
 * the surface is left at alpha 0.
 *
 * Measured in a bare WebKitGTK 2.54 harness with the real guide page, mutating one text node per
 * second: 12.6% of the window painted after six updates under a transparent view, against 100%
 * under an opaque one. Measured on the real analysis window: resizing it to 1100x780 left 1.3%
 * painted. Neither is a CSS fault — the DOM and computed styles are correct in both cases — so no
 * stylesheet change can fix it.
 *
 * Alternating the root element's opacity between `1` and `0.999` forces WebKit to composite the
 * entire layer instead of only the damaged rectangle. The difference is 0.1% and invisible on
 * screen. Measured with this in place: 100% painted after 30 consecutive clock ticks, and across
 * a mode switch and a resize.
 *
 * The values **must** alternate. Re-assigning the same value is not a change, so WebKit does not
 * recomposite — a steady `opacity: 0.999` measured 9.1% painted. Element-level promotion hints
 * (`will-change`, `transform`, `contain`, `backdrop-filter`) were all measured and do not help;
 * the toggle has to be on the root element.
 *
 * See ADR 0019 for the full record.
 */

/** Which of the two near-identical opacities the next nudge should use. */
let flip = false;

/**
 * Forces the next composite to cover the whole view rather than one damaged rectangle.
 *
 * Safe to call at any frequency and from anywhere, but it is only worth calling when something
 * actually changed on screen — it exists to repair *repaints*, not to schedule them. Calling it
 * on a timer with no change would keep the compositor busy for nothing.
 */
export function nudgeRepaint(): void {
	if (typeof document === 'undefined') return;
	flip = !flip;
	document.documentElement.style.opacity = flip ? '0.999' : '1';
}

/**
 * Undoes the inline opacity so a view that is not using the nudge is left untouched.
 *
 * The nudge sets an inline style on `<html>`, which outlives the component that called it. A
 * view that does not repaint would otherwise sit at 99.9% opacity for no reason.
 */
export function clearRepaintNudge(): void {
	if (typeof document === 'undefined') return;
	document.documentElement.style.opacity = '';
}

/**
 * Repairs a repaint that a `:hover` (or `:focus`) restyle is about to cause.
 *
 * `nudgeRepaint` inside a pointer listener is too EARLY for hover. The listener runs on the event
 * that moves the pointer, but the style change it triggers (`:hover` matching the element now under
 * the cursor) is applied in the frame *after* the event is dispatched. So the sequence on a real
 * window is: pointermove → our nudge composites the whole layer → WebKit applies the hover style,
 * invalidates only the button's rectangle, discards the backing store → the window is left showing
 * nothing but that button.
 *
 * Measured on the analysis window with the buttons in the OLD layout: coverage fell from 100% to
 * 0.1% on every hover-in, recovering only when something else forced a composite. The same
 * measurement through the restructured layout came out 100% on every trial, so the collapse is not
 * happening any more — but the ordering hazard is real and this is what removes it rather than
 * relying on an unrelated layout detail to keep it away.
 *
 * Deferring to the next animation frame puts the repair after the hover style has been applied:
 * `requestAnimationFrame` callbacks run after style recalc and layout for that frame, so the forced
 * composite includes the restyled button and the discarded store is replaced in the same frame the
 * damage happened in. The second nudge, one frame later, covers the case where WebKit applies the
 * hover paint in the frame after the style change.
 *
 * Alternating values are preserved because `nudgeRepaint` owns the toggle, so two calls in a row are
 * two real changes — which is what makes WebKit composite rather than short-circuit.
 */
function nudgeAfterHoverRepaint(): void {
	if (typeof document === 'undefined' || typeof requestAnimationFrame !== 'function') return;
	requestAnimationFrame(() => {
		nudgeRepaint();
		requestAnimationFrame(nudgeRepaint);
	});
}

/**
 * Calls `nudgeRepaint` after every layout change that WebKit should have repainted.
 *
 * A `ResizeObserver` on the document element covers the case the user actually reported on the
 * analysis window: interacting with it — resizing, scrolling, changing route — repaints a
 * region, the backing store is discarded, and everything outside that region goes invisible.
 * The observer fires for the layout changes that cause it without the app having to guess which
 * interactions matter.
 *
 * @param win The window to observe, defaulting to the current one.
 * @returns A function that stops observing.
 */
export function observeRepaints(win: Window = window): () => void {
	if (typeof document === 'undefined') {
		return () => {};
	}
	const teardown: Array<() => void> = [];

	// The first composite can come out empty, before anything has changed: measured on the
	// analysis view, one launch in five (and later two in six) came up at 99.6% alpha-0 and
	// stayed there until the player touched the window. Nothing else covers that, because a
	// repaint can only be repaired by a repaint — so the opening seconds are nudged explicitly.
	// Two nudges at different distances: the first may land before the page has laid out, and
	// the second settles the case where the first one won.
	const settle = [400, 1400, 3000].map((delay) => setTimeout(nudgeRepaint, delay));
	teardown.push(() => settle.forEach(clearTimeout));

	// A periodic nudge while the app is on screen. This is the safety net under all of the above:
	// the fault is that *any* repaint can discard the backing store, and the interactions that
	// cause one are not enumerable (scrolling, route changes, focus changes, content updates).
	// One style assignment every couple of seconds is far cheaper than an invisible window, and it
	// is invisible itself — the two values differ by 0.1%.
	//
	// The interval is a *repair*, not a prevention: measured, a drag-selection dropped the window
	// to 12.7% painted and the next tick brought it back, which leaves it blank for up to two
	// seconds. That latency is the user-visible bug, so the interval is the floor and the
	// per-frame loop below is what actually keeps it painted.
	const timer = setInterval(nudgeRepaint, 2000);
	teardown.push(() => clearInterval(timer));

	// Nudge on the input events that cause damage-rect repaints. This is the trigger that matters
	// for hover and selection, which is what the user reported.
	//
	// Both are the same fault: the pointer moves, WebKit invalidates only the few pixels under it
	// (`:hover` on a link, or the dragged selection) and discards the backing store, so the window
	// collapses to just those pixels and everything else goes see-through. Measured on the real
	// window: a drag-selection across body text dropped it to 12.7% painted, and the 2-second
	// interval brought it back — leaving it blank for up to two seconds, which is the visible bug.
	//
	// Input listeners run before the frame is composited and after the damage that triggered them,
	// so nudging here puts the repair in the same frame as the repaint that damaged it: never
	// visibly blank. That is why these events and not a timer.
	//
	// Deliberately not a `requestAnimationFrame` pump. A continuous per-frame loop was measured to
	// work too, but it cost 3% of a core while the app sat idle, which is a poor trade for a
	// resident app. These listeners cost nothing until the player actually touches the window.
	for (const type of ['pointermove', 'pointerdown', 'pointerup', 'wheel', 'keydown', 'scroll']) {
		const onInput = () => nudgeRepaint();
		// `passive` because none of these handlers prevent anything; capture so an element that
		// stops propagation cannot hide the damage from this repair.
		win.addEventListener(type, onInput, { passive: true, capture: true });
		teardown.push(() => win.removeEventListener(type, onInput, { capture: true }));
	}

	// Hover and focus changes need the deferred repair, not the immediate one above: those style
	// changes are applied in the frame after the event, so an immediate nudge lands first and the
	// damage wins. `pointerover`/`pointerout` are the events that actually fire when the hovered
	// element changes (a `pointermove` within one element fires neither); `focusin`/`focusout` cover
	// the same hazard for a keyboard user tabbing onto a control, which restyles it just the same.
	//
	// Both immediate AND deferred, deliberately: the immediate one keeps the pointermove behaviour
	// that is already known to work for selection and scrolling, and the deferred one is what the
	// hover restyle needs. Firing both costs two style assignments.
	for (const type of ['pointerover', 'pointerout', 'focusin', 'focusout']) {
		const onRestyle = () => {
			nudgeRepaint();
			nudgeAfterHoverRepaint();
		};
		win.addEventListener(type, onRestyle, { passive: true, capture: true });
		teardown.push(() => win.removeEventListener(type, onRestyle, { capture: true }));
	}

	if (typeof ResizeObserver !== 'undefined') {
		const observer = new ResizeObserver(() => nudgeRepaint());
		observer.observe(document.documentElement);
		teardown.push(() => observer.disconnect());
	}

	// A route change replaces the page content without necessarily resizing the document, so the
	// navigation that made the analysis view vanish is not always covered by the observer.
	const onNav = () => nudgeRepaint();
	win.addEventListener('popstate', onNav);
	teardown.push(() => win.removeEventListener('popstate', onNav));

	return () => {
		for (const stop of teardown) stop();
		clearRepaintNudge();
	};
}

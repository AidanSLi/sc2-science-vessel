/**
 * Which view this window renders.
 *
 * The app has **two windows**, one per mode (ADR 0028): the analysis window and the overlay
 * window. Both load the same page, because both are the same frontend, so each has to know which
 * view to render — otherwise the overlay window would draw the analysis page inside a 380x600
 * frame, and the analysis window would draw a click-through guide.
 *
 * The answer comes from Tauri, which gives every window a **label** and makes it available to the
 * page through `getCurrentWindow().label`. That is the honest source: the label is what the shell
 * uses to find a window, so deriving the view from it means the two can never disagree.
 *
 * The shell declares the labels in `tauri.conf.json` and in `lib.rs` (`MAIN_WINDOW_LABEL`,
 * `OVERLAY_WINDOW_LABEL`); the constants here must match. A mismatch is not silent — an
 * unrecognised label resolves to the analysis view, so the app stays usable, but the guide would
 * never appear.
 *
 * This is distinct from `windowMode` in `#lib/window-mode`, which mirrors the *shell's* mode and
 * says which window is currently on screen. The two agree in practice, but they answer different
 * questions: this one is fixed for the life of a window and decides what to render; that one
 * changes and decides what the shell should do next. Deriving the view from the mode would make a
 * window re-render as the other one's view, which is exactly what two windows exist to prevent.
 */

/** Which of the app's two views a window renders. */
export type WindowView = 'analysis' | 'overlay';

/** The window label that renders the overlay. Must match `OVERLAY_WINDOW_LABEL` in `lib.rs`. */
export const OVERLAY_WINDOW_LABEL = 'overlay';

/**
 * Narrows an unknown value to a `WindowView`.
 *
 * Anything unrecognised resolves to `'analysis'`, the harmless view: a mistyped label must not
 * produce a click-through text-only panel where the player expects the app.
 *
 * @param value The raw value to interpret.
 * @returns The matching view, or `null` if the value is not a view at all.
 */
export function parseWindowView(value: unknown): WindowView | null {
	return value === 'overlay' || value === 'analysis' ? value : null;
}

/**
 * Maps a window label to the view that window renders.
 *
 * Only the overlay label is special; every other label — `main`, and the empty string a plain
 * browser tab reports — is the analysis view.
 *
 * @param label The window's label, as Tauri reports it.
 * @returns The view that window renders.
 */
export function viewForLabel(label: string | null | undefined): WindowView {
	return label === OVERLAY_WINDOW_LABEL ? 'overlay' : 'analysis';
}

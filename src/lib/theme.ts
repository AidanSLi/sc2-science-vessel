/**
 * The app's visual theme, and how it gets applied to the document.
 *
 * A theme is a set of CSS custom-property overrides keyed on `html[data-theme]` (see the block
 * in `src/app.css`), so switching one is a single attribute write and needs no component to
 * re-render. That matters more than it looks: the overlay is a transparent window over a live
 * game, and a theme switch that repainted the whole tree would produce exactly the kind of
 * in-place redraw ADR 0019 measured as unreliable under a transparent WebKitGTK view.
 *
 * The chosen theme is persisted by the shell in `settings.json` alongside the other settings,
 * rather than in `localStorage`, for the same reason settings live in Rust at all: the value has
 * to be read before the first paint, and a webview that started on the default palette and
 * switched afterwards would flash the wrong background every launch.
 */

import { isTauri } from './client-api';

/**
 * The themes offered in Settings, in the order they are listed.
 *
 * `terran` is first and is the fallback for anything unrecognised, because it is the neutral,
 * low-chroma palette the app was designed against — a bad or missing theme value has to degrade
 * to something readable rather than to a half-applied set of variables.
 */
export const THEMES = [
	{
		id: 'terran',
		label: 'Terran',
		description: 'Green console panels with a gunmetal frame, after the Terran briefing screens.'
	},
	{
		id: 'zerg',
		label: 'Zerg',
		description: 'Violet carapace panels edged in rust, after the Zerg hive consoles.'
	},
	{
		id: 'protoss',
		label: 'Protoss',
		description: 'Light-blue energy panels with gold plating, after the Protoss nexus console.'
	},
	{
		id: 'space',
		label: 'Space',
		description: 'Cyan instruments over deep space, with drifting stars and slow nebulae.'
	}
] as const;

/** The identifier of a selectable theme. */
export type ThemeId = (typeof THEMES)[number]['id'];

/** The theme used when nothing has been chosen, matching the Rust default. */
export const DEFAULT_THEME: ThemeId = 'terran';

/**
 * Narrows an arbitrary value to a known theme id.
 *
 * Defensive because the value crosses two boundaries — a settings file the user may have
 * hand-edited, and an IPC reply — and an unknown id would otherwise be written into
 * `data-theme`, leaving every theme-scoped rule unmatched and the app on unstyled fallbacks.
 *
 * @param value The raw value to interpret.
 * @returns The matching theme id, or the default.
 */
export function parseTheme(value: unknown): ThemeId {
	return THEMES.some((theme) => theme.id === value) ? (value as ThemeId) : DEFAULT_THEME;
}

/**
 * Whether the theme draws an animated sky behind the content.
 *
 * Only Space does. Kept as a function rather than a flag on the theme list so the rule lives in
 * one place when a second ambient theme is added.
 *
 * @param theme The active theme.
 * @returns True when the layout should render the starfield layer.
 */
export function themeHasSky(theme: ThemeId): boolean {
	return theme === 'space';
}

/**
 * Whether this platform can animate the ambient background without burning a CPU core.
 *
 * The answer is a *platform* fact, not a CSS one, which is why the shell reports it rather than
 * the stylesheet deciding. Linux runs WebKitGTK on the software compositor this app is forced
 * onto (ADR 0005, ADR 0022), where a continuously animating element measures 100% of a core
 * whatever its size — six 4x4px dots cost the same as two full-window gradient layers — because
 * every frame repaints the whole window on the CPU (ADR 0021). Windows runs WebView2 over
 * Chromium's compositor, where a transform or opacity animation is a GPU layer and costs
 * approximately nothing.
 *
 * Read once from the shell and cached, because it is asked on every theme change and never
 * changes within a process. Outside the desktop shell — the frontend is also served in a plain
 * browser during development — the answer is the safe one, so a developer on Linux sees the
 * shipping behaviour rather than a locals-only animation.
 *
 * @returns True when the theme's ambient layers may animate.
 */
export async function animationsSupported(): Promise<boolean> {
	if (!isTauri()) return false;
	try {
		const { invoke } = await import('@tauri-apps/api/core');
		return await invoke<boolean>('animations_supported_command');
	} catch {
		// A shell too old to answer predates the command, and every such build is a Linux one.
		return false;
	}
}

/**
 * Applies a theme to the document by setting `data-theme` on `<html>`.
 *
 * Also mirrors the theme's darkest surface into the webview's own background colour, so the
 * pixels behind the page match the page. That matters on the transparent window: while the page
 * is still loading there is a frame or two where nothing has been painted yet, and without this
 * the window shows the previous theme's colour (or the desktop) before snapping to the new one.
 *
 * @param theme The theme to apply.
 */
export function applyTheme(theme: ThemeId): void {
	if (typeof document === 'undefined') return;
	applyMotionedTheme(document.documentElement, theme);
}

/**
 * Whether the ambient motion has been confirmed to be affordable on this machine.
 *
 * Set once by `initAnimations` and read by `applyTheme`. Deliberately a module-level flag rather
 * than an event listener on every caller: `data-motion` has to be correct for the *first* themed
 * frame, and a second async reader per call site would reintroduce the flash that writing the
 * default theme into `app.html` exists to avoid.
 */
let motionEnabled = false;

/**
 * Asks the shell whether animation is affordable and records the answer for `applyTheme`.
 *
 * Must be awaited before the first `applyTheme` call, which `initTheme` does.
 *
 * @returns True when ambient animation is enabled.
 */
export async function initAnimations(): Promise<boolean> {
	motionEnabled = await animationsSupported();
	return motionEnabled;
}

/**
 * Writes the theme and the motion capability onto a root element.
 *
 * Split out from `applyTheme` so the two attributes are set together and in one place: a theme
 * applied without the matching `data-motion` is the bug this shape prevents — it would show a
 * static Space background on a platform that could animate it, or an animating one on a platform
 * that cannot afford it.
 *
 * @param root The document element.
 * @param theme The theme to apply.
 */
function applyMotionedTheme(root: HTMLElement, theme: ThemeId): void {
	if (root.dataset.theme === theme && root.dataset.motion) return;
	root.dataset.theme = theme;
	// `data-motion` gates the CSS animations. Written as `on`/`off` rather than added and removed
	// because the attribute ends up in selectors either way, and this cannot leave a stale value
	// behind from a previous theme.
	root.dataset.motion = motionEnabled ? 'on' : 'off';
	// Components that render theme-dependent *markup* — the root layout decides whether to mount
	// the ambient sky layers — cannot see a custom-property change, because nothing about them
	// re-renders. The attribute alone is enough for CSS, but not for the DOM, so the change is
	// also announced. Guarded by the check above so re-applying the same theme (which `initTheme`
	// does on every mount) does not fire a redundant event.
	document.dispatchEvent(new CustomEvent(THEME_CHANGE_EVENT, { detail: theme }));
}

/** The event name fired when the applied theme changes. See `applyTheme`. */
export const THEME_CHANGE_EVENT = 'sc2sv:theme-change';

/**
 * Subscribes to theme changes.
 *
 * @param listener Called with the newly applied theme.
 * @returns An unsubscribe function.
 */
export function onThemeChange(listener: (theme: ThemeId) => void): () => void {
	const handler = (event: Event) => {
		listener(parseTheme((event as CustomEvent).detail));
	};
	document.addEventListener(THEME_CHANGE_EVENT, handler);
	return () => document.removeEventListener(THEME_CHANGE_EVENT, handler);
}

/**
 * Loads the stored theme and applies it.
 *
 * Called once from the root layout before the first paint. Outside the desktop shell — the
 * frontend is also served directly in a browser during development — the default theme is
 * applied and nothing is persisted.
 *
 * @returns The theme that was applied.
 */
export async function initTheme(): Promise<ThemeId> {
	// Before the palette, because `applyTheme` writes `data-motion` alongside `data-theme` and the
	// CSS decides whether to animate from it. Awaiting here keeps the first themed frame correct.
	await initAnimations();
	let theme: ThemeId = DEFAULT_THEME;
	if (isTauri()) {
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			theme = parseTheme(await invoke<string>('get_theme'));
		} catch {
			// A missing or unreadable settings file is the first-run case, not an error worth
			// surfacing: the default theme is a complete, working appearance.
			theme = DEFAULT_THEME;
		}
	}
	applyTheme(theme);
	return theme;
}

/**
 * Stores a theme choice and applies it.
 *
 * The attribute is written before the round trip completes so the UI responds immediately —
 * a theme picker that waited on disk I/O before changing colour would feel broken — and the
 * shell's reply is then applied over the top, so a refused write cannot leave the screen
 * showing a theme that was not saved.
 *
 * @param theme The requested theme.
 * @returns The theme the shell stored, which is the requested one unless it was rejected.
 * @throws When the shell is unavailable or refused to persist the choice.
 */
export async function saveTheme(theme: ThemeId): Promise<ThemeId> {
	const chosen = parseTheme(theme);
	applyTheme(chosen);
	if (!isTauri()) {
		throw new Error('The desktop app is needed to remember the theme.');
	}
	const { invoke } = await import('@tauri-apps/api/core');
	const stored = parseTheme(await invoke<string>('set_theme', { theme: chosen }));
	applyTheme(stored);
	return stored;
}

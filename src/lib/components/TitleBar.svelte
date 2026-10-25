<script lang="ts">
	/**
	 * The app's own title bar, drawn flush against the content.
	 *
	 * The window is declared `decorations: false` and this replaces the native chrome, because a
	 * toolkit-drawn title bar follows the *desktop's* theme and cannot follow one of ours: a
	 * grey headerbar sitting above a Terran console panel is the exact seam this was written to
	 * remove. Everything here — the drag handle, the window buttons, the resize edges — is page
	 * markup styled by the theme tokens, so it changes with the theme like the rest of the app.
	 *
	 * Three behaviours have to be provided by hand once the decoration is gone, and each is
	 * easy to get subtly wrong:
	 *
	 * 1. **Dragging.** A non-interactive `data-tauri-drag-region` strip makes the window move.
	 * 2. **Resizing.** A frameless window has no toolkit resize border, so eight invisible edges
	 *    call `startResizeDragging`. They are siblings of the drag region rather than children,
	 *    because a resize handle inside a drag region would start a move instead of a resize.
	 * 3. **Focus.** Clicking any of it must not be swallowed by the drag region, so the buttons
	 *    opt out with `data-tauri-drag-region="false"`.
	 *
	 * The bar is hidden in overlay mode: a build guide pinned over a game has `decorations: false`
	 * already and must not grow a title bar, so this component is only mounted by the analysis
	 * layout.
	 */
	import { onMount, type Snippet } from 'svelte';
	import { isTauri } from '#lib/client-api';

	/** The window title shown on the left, matching the desktop file's name. */
	const TITLE = 'SC2 Science Vessel';

	/** Whether the window is currently maximised, for the button's icon and label. */
	let maximised = $state(false);

	/**
	 * The eight resize handles, as [direction, cursor, position classes].
	 *
	 * `nwse-resize` on the diagonals and `ns`/`ew` on the edges is the convention every desktop
	 * uses, so the cursor alone tells the user what the handle does.
	 */
	const handles = [
		['North', 'cursor-ns-resize', 'top-0 left-2 right-2 h-1'],
		['South', 'cursor-ns-resize', 'bottom-0 left-2 right-2 h-1'],
		['West', 'cursor-ew-resize', 'left-0 top-2 bottom-2 w-1'],
		['East', 'cursor-ew-resize', 'right-0 top-2 bottom-2 w-1'],
		['NorthWest', 'cursor-nwse-resize', 'top-0 left-0 h-3 w-3'],
		['NorthEast', 'cursor-nesw-resize', 'top-0 right-0 h-3 w-3'],
		['SouthWest', 'cursor-nesw-resize', 'bottom-0 left-0 h-3 w-3'],
		['SouthEast', 'cursor-nwse-resize', 'bottom-0 right-0 h-3 w-3']
	] as const;

	/**
	 * Asks the window manager to start a resize drag from a handle.
	 *
	 * Failures are swallowed: outside the desktop shell there is no window to resize, and in the
	 * shell a refused resize is the window manager's decision rather than something the user can
	 * act on.
	 *
	 * @param direction One of Tauri's `ResizeDirection` variants.
	 */
	async function startResize(direction: string) {
		if (!isTauri()) return;
		try {
			const { getCurrentWindow } = await import('@tauri-apps/api/window');
			await getCurrentWindow().startResizeDragging(direction as never);
		} catch {
			// Nothing to report: the window simply does not resize on this path.
		}
	}

	/**
	 * Runs a window action and refreshes the maximised flag afterwards.
	 *
	 * The flag is re-read rather than toggled locally, because the window manager can change the
	 * state on its own (double-clicking the drag region, a keyboard shortcut, a tiling gesture),
	 * and a locally-toggled icon would then disagree with the actual window.
	 *
	 * @param action The window method to call.
	 */
	async function act(action: 'minimize' | 'toggleMaximize' | 'close') {
		if (!isTauri()) return;
		try {
			const { getCurrentWindow } = await import('@tauri-apps/api/window');
			const win = getCurrentWindow();
			if (action === 'close') await win.close();
			else if (action === 'minimize') await win.minimize();
			else await win.toggleMaximize();
			maximised = await win.isMaximized();
		} catch {
			// As above.
		}
	}

	onMount(() => {
		if (!isTauri()) return;
		let stop: (() => void) | undefined;
		void (async () => {
			try {
				const { getCurrentWindow } = await import('@tauri-apps/api/window');
				const win = getCurrentWindow();
				maximised = await win.isMaximized();
				// Keep the flag honest while the window lives: any resize can change it.
				stop = await win.onResized(() => {
					void win.isMaximized().then((value) => {
						maximised = value;
					});
				});
			} catch {
				// Not fatal: the buttons still work, only the maximize icon can go stale.
			}
		})();
		return () => stop?.();
	});
</script>

{#snippet windowButton(label: string, onclick: () => void, icon: Snippet, danger = false)}
	<button
		type="button"
		aria-label={label}
		title={label}
		data-tauri-drag-region="false"
		class="grid h-7 w-9 place-items-center rounded-sm text-ink-400 transition-colors {danger
			? 'hover:bg-behind/80 hover:text-ink-100'
			: 'hover:bg-surface-700 hover:text-accent'}"
		onclick={onclick}
	>
		{@render icon()}
	</button>
{/snippet}

<!--
	The bar is the theme's SECONDARY colour, washed over the surface, with the bottom rule in the same
	hue at full strength. Using `accent-2` here rather than a neutral `surface-900` is what makes the
	window frame part of the palette: in Terran it is a steel strip, in Protoss a gold one. It is a
	subtle wash (14%) rather than a solid fill so the title bar does not become the brightest thing on
	screen, which a full-strength gold or orange would.

	`sticky top-0`, not a static strip: the window is frameless, so this bar is the ONLY place a
	drag can start, and `.app-surface` — not the document — is the element that scrolls (see the
	comment on it in app.css). A long page therefore used to carry the one drag handle off the top
	of the screen, leaving a window that could not be moved without first scrolling back up. A
	sticky bar keeps the handle under the pointer at every scroll position.

	Two details are required for that to be safe rather than merely clever:

	1. **An opaque background.** It already has one — `color-mix` of two opaque tokens is opaque —
	   so content scrolling underneath cannot show through the bar.
	2. **A stacking context above the page.** `z-20` is above the page content and the sky layers,
	   which are `position: fixed` at z-index auto. Without it, cards scroll over the bar.

	The resize handles stay inside the header, so they travel with it; a frameless window needs its
	edges reachable for the same reason it needs its drag region reachable.
-->
<header
	class="sticky top-0 z-20 flex h-9 shrink-0 select-none items-center gap-3 border-b bg-surface-900 px-3"
	style="background-color: color-mix(in oklab, var(--color-surface-900) 86%, var(--color-accent-2)); border-bottom-color: var(--color-accent-2)"
>
	<!--
		The drag region is the whole bar minus the buttons: `data-tauri-drag-region` makes the
		element a move handle, and children inherit the behaviour, so every interactive child
		opts out explicitly.
	-->
	<!--
		`data-tauri-drag-region="deep"`, not the bare attribute. Tauri's `drag.js` treats a bare
		(or `"true"`) attribute as "self only": it fires a drag only when the *event target itself*
		is the element, so clicking the `<h1>` or the mark inside this strip did nothing — measured,
		a synthetic drag from the middle of the title bar left the window at its exact previous
		origin. `"deep"` accepts a click anywhere in the subtree.

		The double-click is NOT handled here either. The same script already calls
		`internal_toggle_maximize` on `detail === 2`, so a local `ondblclick` toggled it a second
		time and the two cancelled out.

		`role="presentation"`: a window move handle is not a keyboard-operable control. Its
		double-click shortcut is duplicated by the reachable maximise button beside it.
	-->
	<div class="flex min-w-0 flex-1 items-center gap-2" data-tauri-drag-region="deep" role="presentation">
		<!--
			A console-style mark rather than the app icon: three angled bars in the theme's accent
			colour, echoing the status strips on the race consoles. It is drawn in CSS so it
			changes colour with the theme.
		-->
		<span class="flex shrink-0 items-center gap-[2px]" aria-hidden="true">
			<span class="h-4 w-[3px] -skew-x-12 bg-accent"></span>
			<span class="h-3 w-[3px] -skew-x-12 bg-accent/80"></span>
			<span class="h-2.5 w-[3px] -skew-x-12 bg-accent-2"></span>
		</span>
		<h1 class="truncate text-xs font-medium uppercase tracking-[0.18em] text-ink-300">
			{TITLE}
		</h1>
	</div>

	<div class="flex items-center gap-1" data-tauri-drag-region="false">
		{@render windowButton('Minimise', () => void act('minimize'), minimiseIcon)}
		{@render windowButton(maximised ? 'Restore' : 'Maximise', () => void act('toggleMaximize'), maximiseIcon)}
		{@render windowButton('Close', () => void act('close'), closeIcon, true)}
	</div>

	<!-- Resize handles, siblings of the drag region so a drag on an edge resizes, not moves. -->
	{#each handles as [direction, cursor, position] (direction)}
		<div
			class="absolute {cursor} {position}"
			data-tauri-drag-region="false"
			onpointerdown={(event) => {
				event.preventDefault();
				void startResize(direction);
			}}
			role="presentation"
		></div>
	{/each}
</header>

{#snippet minimiseIcon()}
	<svg viewBox="0 0 10 10" class="h-2.5 w-2.5" aria-hidden="true">
		<line x1="2" y1="5" x2="8" y2="5" stroke="currentColor" stroke-width="1.2" />
	</svg>
{/snippet}

{#snippet maximiseIcon()}
	<svg viewBox="0 0 10 10" class="h-2.5 w-2.5" aria-hidden="true">
		{#if maximised}
			<rect x="1.5" y="3.5" width="5" height="5" fill="none" stroke="currentColor" stroke-width="1.1" />
			<path d="M3.5 3.5V1.5H8.5V6.5H6.5" fill="none" stroke="currentColor" stroke-width="1.1" />
		{:else}
			<rect x="2" y="2" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1.1" />
		{/if}
	</svg>
{/snippet}

{#snippet closeIcon()}
	<svg viewBox="0 0 10 10" class="h-2.5 w-2.5" aria-hidden="true">
		<path d="M2.5 2.5L7.5 7.5M7.5 2.5L2.5 7.5" stroke="currentColor" stroke-width="1.2" />
	</svg>
{/snippet}

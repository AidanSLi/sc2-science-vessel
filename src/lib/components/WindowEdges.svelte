<script lang="ts">
	/**
	 * The window's own resize border, drawn against the VIEWPORT.
	 *
	 * The window is frameless, so no toolkit resize border exists and this component is it. The
	 * crucial property is that the handles are anchored to the viewport, not to any page box:
	 * `startResizeDragging` names a side of the WINDOW, so a handle positioned relative to anything
	 * smaller than the window resizes an edge that is nowhere near the pointer.
	 *
	 * That is exactly the bug this replaced. The eight handles used to be absolutely positioned
	 * inside the 36px title bar, so the `South` handle lay across the bar's bottom edge while
	 * `'South'` resized the window's bottom: dragging the bottom of the title bar resized the bottom
	 * of the window, from a strip hundreds of pixels away from where the pointer was going. `North`,
	 * `West` and `East` happened to coincide with the window's own edges and hid the mistake.
	 *
	 * Two further things put this component here rather than in `TitleBar.svelte`:
	 *
	 * 1. The title bar is `sticky top-0` (ADR 0025) — always at the top of the viewport, with the
	 *    page scrolling under it — whereas a window edge never moves. Content inside the bar cannot
	 *    represent a window edge, whatever it is positioned against.
	 * 2. Only the analysis view has a window the player can resize. Overlay mode is a 380x600
	 *    override-redirect panel the app positions itself (ADR 0016), so it must not grow a resize
	 *    border; the root layout renders this component only in the analysis branch.
	 *
	 * There is deliberately no drag strip along the top edge. The title bar underneath it is already
	 * the move surface and spans the full width, so a second strip would only be somewhere for the
	 * north edge and the north corners to collide with.
	 */
	import { isTauri } from '#lib/client-api';
	import { windowResizeEdges } from '#lib/window-edges';

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
</script>

<!--
	`position: fixed` is load-bearing, not stylistic: it resolves against the viewport, which for a
	frameless window IS the window, so `bottom-0` means the window's bottom edge and the direction
	handed to the window manager matches where the pointer actually is.

	`z-index` above the page and above the sticky title bar, so an edge is never covered by content
	that happens to reach the window boundary.

	`data-tauri-drag-region="false"` because a pointerdown here must resize, not move the window.
-->
{#each windowResizeEdges as edge (edge.direction)}
	<div
		class="window-resize-edge {edge.cursor}"
		data-placement={edge.placement}
		data-tauri-drag-region="false"
		onpointerdown={(event) => {
			event.preventDefault();
			void startResize(edge.direction);
		}}
		role="presentation"
	></div>
{/each}

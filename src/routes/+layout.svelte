<script lang="ts">
	/**
	 * Root layout: the only place global CSS is imported, so every route (including the
	 * overlay) inherits the theme tokens without importing them twice.
	 */
	import '../app.css';

	let { children } = $props();
</script>

{@render children()}

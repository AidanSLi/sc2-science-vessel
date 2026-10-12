<script lang="ts">
	/**
	 * Match list. Until the sidecar exists this is an empty state that explains itself,
	 * rather than a fake list of games: the one thing this screen must never do is imply
	 * it has read a replay it has not.
	 */
	import { formatGameTime } from '#lib/clock';

	/** Replay folders the user has registered. Persisted by the sidecar in Phase 1. */
	let replayRoots = $state<string[]>([]);
	let newRoot = $state('');

	/** Adds a path the user typed, so a library can be registered before the sidecar lands. */
	function addRoot() {
		const value = newRoot.trim();
		if (!value) return;
		replayRoots = [...replayRoots, value];
		newRoot = '';
	}

	/** A rough count of games, purely to size the empty state. Never fabricated. */
	const gameCount = $state(0);
</script>

<section class="space-y-8">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold">Matches</h1>
			<p class="tabular mt-1 text-sm text-ink-500">
				{gameCount} games indexed{gameCount > 0 ? ` · ${formatGameTime(0)} analysed` : ''}
			</p>
		</div>
		<button
			type="button"
			class="rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-300 hover:bg-surface-700"
			disabled
		>
			Rescan library
		</button>
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-6">
		<h2 class="text-sm font-semibold">Replay library</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			Register a folder of replays and the analysis pipeline will index it. Parsing is fast
			enough to scan a whole library on demand — measured at about 3 ms per replay across 722
			replays — so there is no need for a background service.
		</p>

		<div class="mt-4 flex flex-wrap gap-2">
			<label class="sr-only" for="replay-root">Replay folder path</label>
			<input
				id="replay-root"
				type="text"
				bind:value={newRoot}
				placeholder="/path/to/StarCraft II/Replays"
				class="min-w-72 flex-1 rounded border border-surface-700 bg-surface-950 px-3 py-1.5 text-sm text-ink-100 placeholder:text-ink-500"
				onkeydown={(e) => e.key === 'Enter' && addRoot()}
			/>
			<button
				type="button"
				class="rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-300 hover:bg-surface-700"
				onclick={addRoot}
			>
				Add folder
			</button>
		</div>

		{#if replayRoots.length > 0}
			<ul class="mt-4 space-y-1">
				{#each replayRoots as root (root)}
					<li class="tabular rounded bg-surface-950 px-3 py-1.5 text-xs text-ink-300">{root}</li>
				{/each}
			</ul>
		{/if}

		<p class="mt-4 text-xs text-ink-500">
			Folder picking and indexing land with the Python sidecar (Phase 1). Nothing is read
			from disk yet.
		</p>
	</div>
</section>

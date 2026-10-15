<script lang="ts">
	/**
	 * The replay library: every game the app can see, and the entry point to analysis.
	 *
	 * The list is built from the cheap header scan (~3 ms per replay, measured across 783), so
	 * the whole library can be shown without an indexing service or a database. That is why
	 * this page fetches headers rather than full analyses: metrics are computed when a game is
	 * opened, where the wait is expected and attached to something the user is looking at.
	 *
	 * The page is explicit about the two states it can be in — no folders configured, and no
	 * replays found in the folders that are configured — because those look identical if the
	 * page just renders an empty table, and the remedies are different.
	 */
	import { formatGameTime } from '#lib/clock';
	import { scanLibrary, sidecarUnavailableReason, type ReplayHeaderRecord } from '#lib/analysis';
	import { loadSettings, saveReplayRoots, suggestReplayRoots } from '#lib/settings';
	import { raceColorToken } from '#lib/analysis-view';
	import { isTauri } from '#lib/client-api';

	let roots = $state<string[]>([]);
	let suggested = $state<string[]>([]);
	let newRoot = $state('');
	let records = $state<ReplayHeaderRecord[]>([]);
	let loading = $state(false);
	let progress = $state<string | null>(null);
	let error = $state<string | null>(null);
	let settingsLoaded = $state(false);

	/** Free-text filter over map, matchup and player names. */
	let filter = $state('');

	const unavailable = sidecarUnavailableReason();

	// Settings are read once on mount. `onMount` rather than a top-level await because the page
	// is prerendered as a shell (see `+layout.ts`) and must render without the shell present.
	$effect(() => {
		void (async () => {
			const settings = await loadSettings();
			roots = settings.replayRoots;
			suggested = await suggestReplayRoots();
			settingsLoaded = true;
			if (roots.length > 0) await rescan();
		})();
	});

	/**
	 * Scans every configured folder and replaces the table contents.
	 *
	 * A folder that fails is reported and skipped rather than aborting the whole scan: one
	 * unmounted drive should not hide the replays on the other folders. The empty result is
	 * kept distinguishable from a failure by `error` carrying the reason.
	 */
	async function rescan() {
		if (unavailable || roots.length === 0) return;
		loading = true;
		error = null;
		const collected: ReplayHeaderRecord[] = [];
		const failures: string[] = [];
		for (const root of roots) {
			progress = `Reading ${root}…`;
			try {
				const found = await scanLibrary(root);
				collected.push(...found);
			} catch (err) {
				failures.push(`${root}: ${err instanceof Error ? err.message : String(err)}`);
			}
		}
		records = collected;
		error = failures.length > 0 ? failures.join('; ') : null;
		progress = null;
		loading = false;
	}

	/** Persists the current folder list and rescans. */
	async function persistAndScan() {
		try {
			roots = await saveReplayRoots(roots);
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
			return;
		}
		await rescan();
	}

	/** Adds a typed path. */
	async function addRoot(path: string) {
		const value = path.trim();
		if (!value || roots.includes(value)) return;
		roots = [...roots, value];
		newRoot = '';
		await persistAndScan();
	}

	/** Removes a folder, leaving the files alone. */
	async function removeRoot(path: string) {
		roots = roots.filter((root) => root !== path);
		await persistAndScan();
	}

	/** Replays read successfully, which is what every count on this page is about. */
	const parsed = $derived(records.filter((record) => record.error === null));

	/** The visible rows, newest first. */
	const rows = $derived.by(() => {
		const needle = filter.trim().toLowerCase();
		const matching = needle
			? parsed.filter((record) =>
					[record.map_name, record.matchup, record.game_type, ...record.players.map((p) => p.name)]
						.join(' ')
						.toLowerCase()
						.includes(needle)
				)
			: parsed;
		return [...matching].sort((a, b) => (b.date ?? '').localeCompare(a.date ?? ''));
	});

	/** Totals shown in the header, computed over parsed replays only. */
	const totals = $derived.by(() => {
		const seconds = parsed.reduce((sum, record) => sum + record.lengthSeconds, 0);
		const ladder = parsed.filter((record) => record.is_ladder).length;
		const matchups = new Map<string, number>();
		for (const record of parsed) {
			if (record.matchup) matchups.set(record.matchup, (matchups.get(record.matchup) ?? 0) + 1);
		}
		const topMatchup = [...matchups.entries()].sort((a, b) => b[1] - a[1])[0];
		return { seconds, ladder, topMatchup };
	});

	/** Replays the sidecar could not parse, so an unreadable file is visible rather than absent. */
	const unreadable = $derived(records.filter((record) => record.error !== null));

	/**
	 * The route to a game's analysis page.
	 *
	 * The replay's path is the id, so it has to survive being a URL segment; `encodeURIComponent`
	 * is what keeps a path with spaces, `#` or `?` in a map name intact.
	 *
	 * @param record The replay to open.
	 * @returns The href for the analysis route.
	 */
	function analyzeHref(record: ReplayHeaderRecord): string {
		return `/analyze/${encodeURIComponent(record.path)}`;
	}

	/**
	 * Reports the winner of a game, for the row summary.
	 *
	 * Falls back to the players' `result` field, because replays of games that ended without a
	 * recorded winner (a save/load, an arcade game) have no entry in `winners`.
	 *
	 * @param record The replay.
	 * @returns A human sentence, or an empty string when the replay does not say.
	 */
	function outcome(record: ReplayHeaderRecord): string {
		if (record.winners.length > 0) return `${record.winners.join(' + ')} won`;
		const decided = record.players.find((player) => player.result);
		return decided ? `${decided.name} ${decided.result.toLowerCase()}` : '';
	}
</script>

<section class="space-y-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold">Matches</h1>
			<p class="tabular mt-1 text-sm text-ink-500">
				{#if loading}
					{progress ?? 'Scanning…'}
				{:else if parsed.length === 0}
					No games indexed
				{:else}
					{parsed.length} games · {Math.round(totals.seconds / 60)} minutes played · {totals.ladder} ladder
					{#if totals.topMatchup}· most played {totals.topMatchup[0]}{/if}
				{/if}
			</p>
		</div>
		<button
			type="button"
			class="rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-300 hover:bg-surface-700 disabled:opacity-50"
			onclick={rescan}
			disabled={loading || roots.length === 0}
		>
			Rescan library
		</button>
	</div>

	{#if unavailable}
		<p class="rounded-lg border border-surface-800 bg-surface-900/60 p-4 text-sm text-ink-300">
			{unavailable}
		</p>
	{/if}

	{#if roots.length === 0 && settingsLoaded && !unavailable}
		<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-6">
			<h2 class="text-sm font-semibold">Where are your replays?</h2>
			<p class="mt-2 max-w-2xl text-sm text-ink-300">
				The app reads <code class="text-ink-100">.SC2Replay</code> files from disk and never uploads
				them. On Linux they are usually inside the Wine prefix the game runs from, which is why the
				paths below are searched for you rather than expected as something you already know.
			</p>

			{#if suggested.length > 0}
				<ul class="mt-4 space-y-2">
					{#each suggested as candidate (candidate)}
						<li class="flex flex-wrap items-center gap-2">
							<button
								type="button"
								class="rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-200 hover:bg-surface-700"
								onclick={() => addRoot(candidate)}
							>
								Use this folder
							</button>
							<span class="tabular text-xs text-ink-500">{candidate}</span>
						</li>
					{/each}
				</ul>
			{:else}
				<p class="mt-4 text-sm text-ink-500">
					No StarCraft II replay folder was found automatically. Paste the path to one below.
				</p>
			{/if}
		</div>
	{:else if roots.length > 0}
		<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
			<h2 class="text-sm font-semibold">Replay folders</h2>
			<ul class="mt-3 space-y-1">
				{#each roots as root (root)}
					<li class="flex flex-wrap items-center gap-2">
						<span class="tabular flex-1 text-xs text-ink-300">{root}</span>
						<button
							type="button"
							class="rounded px-2 py-1 text-xs text-ink-500 hover:bg-surface-800 hover:text-ink-300"
							onclick={() => removeRoot(root)}
						>
							Remove
						</button>
					</li>
				{/each}
			</ul>

			<div class="mt-3 flex flex-wrap gap-2">
				<label class="sr-only" for="new-root">Another replay folder</label>
				<input
					id="new-root"
					type="text"
					bind:value={newRoot}
					placeholder="/path/to/StarCraft II/Replays/Multiplayer"
					class="min-w-72 flex-1 rounded border border-surface-700 bg-surface-950 px-3 py-1.5 text-sm text-ink-100 placeholder:text-ink-500"
					onkeydown={(e) => e.key === 'Enter' && addRoot(newRoot)}
				/>
				<button
					type="button"
					class="rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-300 hover:bg-surface-700"
					onclick={() => addRoot(newRoot)}
				>
					Add folder
				</button>
			</div>
			{#if suggested.length > 0}
				<p class="mt-2 text-xs text-ink-500">
					Also found:
					{#each suggested as candidate (candidate)}
						<button
							type="button"
							class="underline decoration-dotted hover:text-ink-300"
							onclick={() => addRoot(candidate)}
						>
							{candidate}
						</button>
					{/each}
				</p>
			{/if}
		</div>
	{/if}

	{#if error}
		<p class="rounded border border-behind/40 bg-surface-900/60 px-3 py-2 text-sm text-ink-300">
			{error}
		</p>
	{/if}

	{#if roots.length > 0 && parsed.length === 0 && !loading && !unavailable}
		<div class="rounded-lg border border-dashed border-surface-700 bg-surface-900/40 p-6 text-sm text-ink-300">
			<p class="font-semibold text-ink-100">No replays found in {roots.length === 1 ? 'that folder' : 'those folders'}.</p>
			<p class="mt-1">
				The scanner looks for files ending in <code class="text-ink-100">.SC2Replay</code>, case
				sensitively, under every subfolder. The game stores them in
				<code class="text-ink-100">Accounts/&lt;account&gt;/&lt;profile&gt;/Replays/Multiplayer</code>, so
				pointing at the <code class="text-ink-100">Accounts</code> folder works too.
			</p>
		</div>
	{/if}

	{#if parsed.length > 0}
		<div class="flex flex-wrap items-center gap-3">
			<label class="sr-only" for="filter">Filter games</label>
			<input
				id="filter"
				type="text"
				bind:value={filter}
				placeholder="Filter by map, matchup or player"
				class="min-w-64 flex-1 rounded border border-surface-700 bg-surface-950 px-3 py-1.5 text-sm text-ink-100 placeholder:text-ink-500"
			/>
			<span class="tabular text-xs text-ink-500">{rows.length} shown</span>
		</div>

		<div class="overflow-x-auto rounded-lg border border-surface-800">
			<table class="w-full text-left text-sm">
				<caption class="sr-only">Replays found in the configured folders, newest first</caption>
				<thead class="bg-surface-900 text-xs text-ink-500 uppercase">
					<tr>
						<th scope="col" class="px-3 py-2 font-medium">Map</th>
						<th scope="col" class="px-3 py-2 font-medium">Matchup</th>
						<th scope="col" class="px-3 py-2 font-medium">Players</th>
						<th scope="col" class="px-3 py-2 text-right font-medium">Length</th>
						<th scope="col" class="px-3 py-2 font-medium">Played</th>
					</tr>
				</thead>
				<tbody>
					{#each rows as record (record.path)}
						<tr class="border-t border-surface-800 hover:bg-surface-900/60">
							<td class="px-3 py-2">
								<a class="underline decoration-dotted hover:text-ink-100" href={analyzeHref(record)}>
									{record.map_name || 'Unknown map'}
								</a>
								<div class="text-xs text-ink-500">
									{record.game_type}{#if record.is_ladder} · ladder{/if} · {record.release}
								</div>
							</td>
							<td class="px-3 py-2 text-ink-300">{record.matchup || '—'}</td>
							<td class="px-3 py-2">
								{#each record.players as player (player.pid)}
									<span class="mr-2 inline-flex items-center gap-1.5">
										<span
											class="inline-block h-2 w-2 rounded-full"
											style="background: var(--color-{raceColorToken(player.race)})"
											aria-hidden="true"
										></span>
										<span class="text-ink-300">{player.name}</span>
									</span>
								{/each}
								<div class="text-xs text-ink-500">{outcome(record)}</div>
							</td>
							<td class="tabular px-3 py-2 text-right text-ink-300">
								{formatGameTime(record.lengthSeconds)}
							</td>
							<td class="tabular px-3 py-2 text-xs text-ink-500">
								{(record.date ?? '').slice(0, 16).replace('T', ' ')}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}

	{#if unreadable.length > 0}
		<details class="rounded-lg border border-surface-800 bg-surface-900/60 p-4 text-sm">
			<summary class="cursor-pointer text-ink-300">
				{unreadable.length} file{unreadable.length === 1 ? '' : 's'} could not be read
			</summary>
			<ul class="mt-2 space-y-1">
				{#each unreadable as record (record.path)}
					<li class="tabular text-xs text-ink-500">{record.path}: {record.error}</li>
				{/each}
			</ul>
		</details>
	{/if}

	{#if !isTauri()}
		<p class="text-xs text-ink-500">
			This page needs the desktop app; a browser cannot read your disk.
		</p>
	{/if}
</section>

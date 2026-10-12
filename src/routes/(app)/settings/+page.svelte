<script lang="ts">
	/**
	 * Settings, plus the client-API diagnostic.
	 *
	 * The diagnostic is the important half: whether the SC2 client exposes its clock on
	 * loopback decides the entire overlay design (ADR 0004), so this page lets the user
	 * answer that question themselves, with the raw response visible.
	 */
	import { onDestroy } from 'svelte';
	import { CLIENT_API_PORT, isTauri, pollClientApi, type LiveState } from '#lib/client-api';

	let live = $state<LiveState>({ status: 'idle', game: null, lastOkAt: null, detail: null });
	let busy = $state(false);

	const timer = setInterval(async () => {
		if (busy) return;
		live = await pollClientApi();
	}, 2000);

	void (async () => {
		live = await pollClientApi();
	})();

	onDestroy(() => clearInterval(timer));

	/** Forces an immediate poll, for when the user has just started the game. */
	async function checkNow() {
		busy = true;
		live = await pollClientApi();
		busy = false;
	}

	/** The overlay toggle key, matching the Rust shell's binding. */
	const shortcut = 'Ctrl+Alt+B';
</script>

<section class="space-y-8">
	<h1 class="text-xl font-semibold">Settings</h1>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">Game clock (SC2 Client API)</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			The overlay follows the game's own clock, read from the client on
			<code class="rounded bg-surface-950 px-1 py-0.5 text-xs">localhost:{CLIENT_API_PORT}</code
			>. The game freezes that clock while paused, which is what makes the guide pause with
			it. It reports the clock and who is playing — nothing about units, supply or buildings,
			and the app is designed never to need more.
		</p>

		<dl class="tabular mt-4 grid gap-2 text-sm sm:grid-cols-[10rem_1fr]">
			<dt class="text-ink-500">State</dt>
			<dd>
				{live.status}{#if !isTauri()} · browser mode{/if}
			</dd>
			<dt class="text-ink-500">Game clock</dt>
			<dd>
				{live.game ? `${live.game.displayTime.toFixed(1)}s` : '—'}
			</dd>
			<dt class="text-ink-500">In a replay</dt>
			<dd>{live.game ? (live.game.isReplay ? 'yes' : 'no') : '—'}</dd>
			<dt class="text-ink-500">Players</dt>
			<dd>
				{live.game?.players.length
					? live.game.players.map((p) => `${p.name} (${p.race})`).join(', ')
					: '—'}
			</dd>
			<dt class="text-ink-500">Last response</dt>
			<dd>{live.lastOkAt ? new Date(live.lastOkAt).toLocaleTimeString() : 'never'}</dd>
			{#if live.detail}
				<dt class="text-ink-500">Detail</dt>
				<dd class="text-ink-300">{live.detail}</dd>
			{/if}
		</dl>

		<button
			type="button"
			class="mt-4 rounded bg-surface-800 px-3 py-1.5 text-sm text-ink-300 hover:bg-surface-700"
			onclick={checkNow}
			disabled={busy}
		>
			{busy ? 'Checking…' : 'Check now'}
		</button>

		{#if !isTauri()}
			<p class="mt-3 text-xs text-ink-500">
				A browser cannot read loopback, so this stays unreachable in a plain tab. Run the
				desktop app (or a game) to get a real answer.
			</p>
		{/if}
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">Overlay</h2>
		<dl class="tabular mt-3 grid gap-2 text-sm sm:grid-cols-[10rem_1fr]">
			<dt class="text-ink-500">Toggle</dt>
			<dd>{shortcut}</dd>
			<dt class="text-ink-500">Behaviour</dt>
			<dd>Always on top, click-through, follows the game clock.</dd>
			<dt class="text-ink-500">Wayland note</dt>
			<dd>
				Tauri's shortcut support is X11-only. On KDE Wayland the binding may not register;
				the overlay can still be opened from the Builds page. A compositor-level overlay is
				planned (ADR 0003).
			</dd>
		</dl>
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">Privacy</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			Replays never leave this machine. The app makes exactly one kind of network request: to
			the game client on loopback. Importing a build snapshot is the only outbound feature,
			it is manual, and its result is stored locally so the app keeps working offline.
		</p>
	</div>
</section>

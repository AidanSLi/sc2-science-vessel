<script lang="ts">
	/**
	 * Settings, plus the client-API diagnostic.
	 *
	 * The diagnostic is the important half: whether the SC2 client exposes its clock on
	 * loopback decides the entire overlay design (ADR 0004), so this page lets the user
	 * answer that question themselves, with the raw response visible.
	 */
	import { onDestroy } from 'svelte';
	import { exactMatchupBuilds } from '#lib/build-library';
	import { CLIENT_API_PORT, isTauri, pollClientApi, type LiveState } from '#lib/client-api';
	import {
		acceleratorFromEvent,
		DEFAULT_OVERLAY_SHORTCUT,
		loadPresets,
		loadSettings,
		normaliseAccelerator,
		saveLaunchAtLogin,
		saveOverlayShortcut,
		savePresets,
		saveStartHidden,
		type PresetMap
	} from '#lib/settings';
	import type { BuildOrder, Matchup } from '#lib/types';

	/**
	 * The matchups worth offering a preset for, in the order they are shown.
	 *
	 * Every ordered pair, not just the player's own race: the app does not know which race the
	 * user plays, and a preset for a matchup they never play is harmless while a missing one is
	 * a build they cannot configure.
	 */
	const presetMatchups: Matchup[] = [
		'ZvT',
		'ZvP',
		'ZvZ',
		'TvZ',
		'TvP',
		'TvT',
		'PvZ',
		'PvT',
		'PvP'
	];

	/**
	 * The bundled library, grouped once by matchup.
	 *
	 * Grouped here rather than calling `buildsForMatchup` per dropdown because that function
	 * falls back to wildcards, and a preset dropdown must list only builds a user could
	 * reasonably pick for that exact matchup — the wildcard fallback is the overlay's job, and
	 * it is already visible as the `First available` option.
	 */
	const buildsByMatchup: Partial<Record<Matchup, BuildOrder[]>> = {};
	for (const matchup of presetMatchups) {
		buildsByMatchup[matchup] = exactMatchupBuilds(matchup);
	}

	let live = $state<LiveState>({
		status: 'idle',
		game: null,
		screens: null,
		lastOkAt: null,
		detail: null
	});
	let busy = $state(false);

	/** The overlay toggle key as registered by the shell, not as requested. */
	let overlayShortcut = $state(DEFAULT_OVERLAY_SHORTCUT);
	/** True while the key recorder is listening for a press. */
	let recording = $state(false);
	/** Message about the last rebind attempt: success, substitution, or failure. */
	let shortcutMessage = $state<string | null>(null);
	let shortcutError = $state(false);

	/** Per-matchup build presets, as stored by the shell. */
	let presets = $state<PresetMap>({});
	/** Message about the last preset change, since a silent save is indistinguishable from a lost one. */
	let presetMessage = $state<string | null>(null);

	/** Whether the app starts hidden and hints when a game begins. The resident default. */
	let startHidden = $state(true);
	/** Whether the app is launched at login. */
	let launchAtLogin = $state(false);
	/** Message about the last startup-setting change. */
	let startupMessage = $state<string | null>(null);
	let startupError = $state(false);

	/**
	 * Applies a change to one of the two startup switches and reports the outcome.
	 *
	 * Both switches can be refused by the shell — launching at login is refused for a build that
	 * is not installed, because the entry it would write points at a development binary. The
	 * value is re-read from the shell after a refusal so the switch cannot be left showing a
	 * state the shell did not accept.
	 *
	 * @param which Which switch changed.
	 * @param value The requested value.
	 */
	async function updateStartup(which: 'startHidden' | 'launchAtLogin', value: boolean) {
		startupMessage = null;
		startupError = false;
		try {
			if (which === 'startHidden') {
				startHidden = await saveStartHidden(value);
				startupMessage = startHidden
					? 'The app will start hidden and hint when a game begins.'
					: 'The app will open with its window visible.';
			} else {
				launchAtLogin = await saveLaunchAtLogin(value);
				startupMessage = launchAtLogin
					? 'The app will start when you log in.'
					: 'The app will no longer start when you log in.';
			}
		} catch (err) {
			// Reflect reality rather than the request: a refused switch must snap back, or the
			// page would claim a setting the shell never stored.
			const settings = await loadSettings();
			startHidden = settings.startHidden;
			launchAtLogin = settings.launchAtLogin;
			startupMessage = err instanceof Error ? err.message : String(err);
			startupError = true;
		}
	}

	/**
	 * Stores one matchup's preset and reports the result.
	 *
	 * The entire map is re-sent rather than a single entry, so clearing a dropdown removes the
	 * key instead of storing an empty id that would match no build and look configured.
	 *
	 * @param matchup The matchup (or wildcard) being set.
	 * @param buildId The chosen build's id, or an empty string to clear it.
	 */
	async function updatePreset(matchup: Matchup, buildId: string) {
		const next: PresetMap = { ...presets };
		if (buildId) {
			next[matchup] = buildId;
		} else {
			delete next[matchup];
		}
		presets = next;
		try {
			presets = await savePresets(next);
			presetMessage = buildId ? `${matchup} set.` : `${matchup} cleared.`;
		} catch (err) {
			presetMessage = `Could not save: ${err instanceof Error ? err.message : String(err)}`;
		}
	}

	const timer = setInterval(async () => {
		if (busy) return;
		live = await pollClientApi();
	}, 2000);

	void (async () => {
		live = await pollClientApi();
		const settings = await loadSettings();
		overlayShortcut = settings.overlayShortcut;
		startHidden = settings.startHidden;
		launchAtLogin = settings.launchAtLogin;
		presets = (await loadPresets()).presets;
	})();

	onDestroy(() => clearInterval(timer));

	/** Forces an immediate poll, for when the user has just started the game. */
	async function checkNow() {
		busy = true;
		live = await pollClientApi();
		busy = false;
	}

	/**
	 * Applies a keypress captured by the recorder.
	 *
	 * A chord that yields no accelerator (a bare modifier, Escape, or punctuation the parser
	 * would not round-trip) is refused with an explanation instead of being sent to the shell
	 * to fail there. The shell's answer is the registered key, which may differ from the one
	 * requested when the combination is already owned by another application; that
	 * substitution is surfaced because a player whose key quietly did not take effect would
	 * press it mid-game and see nothing happen.
	 *
	 * @param event The keydown captured by the recorder.
	 */
	async function recordShortcut(event: KeyboardEvent) {
		event.preventDefault();
		if (event.key === 'Escape') {
			recording = false;
			shortcutMessage = null;
			return;
		}
		const accelerator = acceleratorFromEvent(event);
		if (!accelerator) {
			shortcutMessage = 'Use a modifier with a letter, digit or function key (e.g. Ctrl+Alt+O).';
			shortcutError = true;
			return;
		}
		recording = false;
		shortcutMessage = 'Registering…';
		shortcutError = false;
		try {
			const result = await saveOverlayShortcut(accelerator);
			overlayShortcut = result.shortcut;
			shortcutMessage = result.substituted
				? `${accelerator} is taken by another application, so ${result.shortcut} was registered instead.`
				: `${result.shortcut} is now the overlay toggle.`;
			shortcutError = result.substituted;
		} catch (err) {
			shortcutMessage = `Could not register a shortcut: ${
				err instanceof Error ? err.message : String(err)
			}`;
			shortcutError = true;
		}
	}

	/** Formats the stored accelerator for display in a fixed modifier order. */
	const shortcutDisplay = $derived(normaliseAccelerator(overlayShortcut));
</script>

<svelte:window
	onkeydown={recording
		? (event) => {
				void recordShortcut(event);
			}
		: undefined}
/>

<section class="space-y-8">
	<h1 class="text-xl font-semibold">Settings</h1>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">Startup</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			The app is meant to be resident: it can start hidden, stay out of the way, and tell you
			how to open the overlay the moment a game begins.
		</p>
		<label class="mt-3 flex max-w-2xl items-start gap-3 text-sm">
			<input
				type="checkbox"
				class="mt-1"
				checked={startHidden}
				onchange={(event) => void updateStartup('startHidden', event.currentTarget.checked)}
			/>
			<span>
				Start hidden
				<span class="block text-xs text-ink-500">
					The window stays invisible until you press the toggle key, so finishing a game does
					not put a window in front of the score screen. A popup appears when a game starts to
					remind you of the key.
				</span>
			</span>
		</label>
		<label class="mt-3 flex max-w-2xl items-start gap-3 text-sm">
			<input
				type="checkbox"
				class="mt-1"
				checked={launchAtLogin}
				onchange={(event) => void updateStartup('launchAtLogin', event.currentTarget.checked)}
			/>
			<span>
				Launch at login
				<span class="block text-xs text-ink-500">
					Starts the app with your desktop session, so it is already running when you begin a
					game. Needs an installed build; a development build cannot write a login entry that
					survives its next rebuild.
				</span>
			</span>
		</label>
		{#if startupMessage}
			<p class="mt-3 text-xs {startupError ? 'text-amber-400' : 'text-ink-400'}">{startupMessage}</p>
		{/if}
	</div>

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
				{live.game?.displayTime != null ? `${live.game.displayTime.toFixed(1)}s` : '—'}
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
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			The toggle key works anywhere, including over the game, so it is worth choosing
			something you will not hit by accident.
		</p>
		<dl class="tabular mt-3 grid gap-2 text-sm sm:grid-cols-[10rem_1fr]">
			<dt class="text-ink-500">Toggle key</dt>
			<dd class="flex flex-wrap items-center gap-2">
				<kbd class="rounded border border-surface-700 bg-surface-950 px-2 py-0.5 font-mono text-xs">
					{shortcutDisplay}
				</kbd>
				{#if isTauri()}
					<button
						type="button"
						class="rounded border px-2 py-0.5 text-xs {recording
							? 'border-amber-600 bg-amber-950/40 text-amber-300'
							: 'border-surface-700 bg-surface-800 text-ink-300 hover:bg-surface-700'}"
						onclick={() => {
							recording = true;
							shortcutMessage = null;
						}}
					>
						{recording ? 'Press a key… (Esc to cancel)' : 'Change'}
					</button>
				{/if}
			</dd>
			<dt class="text-ink-500">Behaviour</dt>
			<dd>Always on top, click-through, follows the game clock.</dd>
			<dt class="text-ink-500">Wayland note</dt>
			<dd>
				Tauri's own shortcut support is X11-only, so on Wayland the binding is made through the
				compositor's GlobalShortcuts portal instead (it asks for approval once). If no binding
				stays registered, the window can still be toggled from the Builds page.
			</dd>
		</dl>
		{#if shortcutMessage}
			<p class="mt-3 text-xs {shortcutError ? 'text-amber-400' : 'text-ink-400'}">
				{shortcutMessage}
			</p>
		{/if}
	</div>

	<div class="rounded-lg border border-surface-800 bg-surface-900/60 p-4">
		<h2 class="text-sm font-semibold">Build preset per matchup</h2>
		<p class="mt-2 max-w-2xl text-sm text-ink-300">
			When a game starts the overlay loads the build you chose for that matchup, so there is
			nothing to pick mid-game. This is the only reason the overlay reads the client API
			besides the clock: the races are fixed before the game starts and never change, so
			choosing a fixed list from them is what a paper notepad would do.
		</p>
		<p class="mt-2 max-w-2xl text-xs text-ink-500">
			A wildcard like <code class="rounded bg-surface-950 px-1">ZvX</code> is used when there is no
			exact entry, so one build per race does not have to be entered three times. Leave a matchup
			blank to use the first build in the library for it.
		</p>

		<div class="mt-4 grid gap-3 sm:grid-cols-2">
			{#each presetMatchups as matchup (matchup)}
				<label class="grid gap-1 text-xs text-ink-500">
					{matchup}
					<select
						class="rounded border border-surface-700 bg-surface-950 px-2 py-1 text-sm text-ink-300"
						value={presets[matchup] ?? ''}
						onchange={(event) => updatePreset(matchup, event.currentTarget.value)}
					>
						<option value="">First available</option>
						{#each buildsByMatchup[matchup] ?? [] as candidate (candidate.id)}
							<option value={candidate.id}>{candidate.title}</option>
						{/each}
					</select>
				</label>
			{/each}
		</div>

		{#if presetMessage}
			<p class="mt-3 text-xs text-ink-400">{presetMessage}</p>
		{/if}
		{#if !isTauri()}
			<p class="mt-3 text-xs text-ink-500">
				Presets are stored by the desktop app; a browser tab has nowhere to save them.
			</p>
		{/if}
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

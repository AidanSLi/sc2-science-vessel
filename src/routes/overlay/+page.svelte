<script lang="ts">
	/**
	 * Build-order guide overlay.
	 *
	 * Deliberately the dullest possible thing: a notepad. It shows the predetermined
	 * steps and where the game clock has reached in them, and it does nothing else. It
	 * does not watch the opponent, it does not watch the player, and it never suggests or
	 * reacts — anything that did would be an unfair advantage, so the design rule is that
	 * this view's only input is the game's own clock.
	 *
	 * The clock comes from the SC2 Client API (ADR 0004). When it is unavailable the
	 * overlay says so and offers a manual fallback rather than pretending to track a game
	 * it cannot see.
	 *
	 * The only thing the client API is used for beyond the clock is *which matchup this is*,
	 * so the right preset can be shown without the player choosing it mid-game. Reading the
	 * races is not reacting to anything: they are fixed before the game starts, they never
	 * change, and the guide itself is a fixed list either way. It is the equivalent of the
	 * player having opened the right page in a notebook beforehand.
	 */
	import { onDestroy } from 'svelte';
	import { advance } from '#lib/builds';
	import { resolvePresetBuild } from '#lib/build-library';
	import { formatGameTime, matchupOf } from '#lib/clock';
	import { isTauri, pollClientApi, type LiveState } from '#lib/client-api';
	import { loadPresets } from '#lib/settings';
	import { sampleBuildOrder } from '#lib/sample-build';
	import type { BuildMatchup, BuildOrder, Matchup } from '#lib/types';

	/**
	 * The build being followed.
	 *
	 * Chosen from the matchup preset, falling back to the sample so the overlay always has
	 * something to show. Never set from anything the players *do* — only from the matchup,
	 * which is fixed before the first unit exists.
	 */
	let build = $state<BuildOrder>(sampleBuildOrder);

	/** The matchup the current build was picked for, so the header can say what is loaded. */
	let presetMatchup = $state<BuildMatchup | null>(null);

	/**
	 * Per-matchup build presets, as configured on the settings page.
	 *
	 * Empty means "no preset", which is a normal first-run state, so the overlay falls back
	 * to the sample rather than showing nothing.
	 */
	let presets = $state<Partial<Record<Matchup, string>>>({});

	/** Latest client state. `status` drives every honesty message on screen. */
	let live = $state<LiveState>({
		status: 'idle',
		game: null,
		lastOkAt: null,
		detail: null
	});

	/**
	 * Manual clock used when the client API is unreachable. Advanced by the +/- buttons
	 * (and the space bar), because a Wayland-friendly global hotkey does not exist yet.
	 */
	let manualClock = $state(0);
	let usingManual = $state(false);

	/**
	 * How opaque the panel behind the text is, as a percentage.
	 *
	 * The window is transparent and the game shows through, which is what the player asked
	 * for, but text over a bright or busy part of the map can be hard to read. The default is
	 * fully transparent on purpose; this exists so the background can be nudged towards a
	 * light translucent panel without a code change, which was an explicitly anticipated
	 * follow-up rather than a guess. `0` means no background at all.
	 */
	let panelOpacity = $state(0);

	/**
	 * Picks the clock source from the latest poll.
	 *
	 * The game's clock always wins when it is readable, and the manual clock takes over
	 * whenever it is not — before the game has started, after it exits, or in a browser tab.
	 * Manual mode is entered automatically rather than only on the first keypress, because
	 * an overlay that shows a frozen `0:00` and no affordance looks broken, while one that
	 * shows "press space" is self-explanatory.
	 *
	 * @param state The freshly polled client state.
	 */
	function applyLiveState(state: LiveState) {
		live = state;
		if (state.status === 'live' || state.status === 'replay') {
			usingManual = false;
		} else if (state.status === 'unavailable' || state.status === 'error') {
			usingManual = true;
		}
	}

	/** Poll the client every second. Locally bound so it can be cleared on destroy. */
	const timer = setInterval(async () => {
		applyLiveState(await pollClientApi());
	}, 1000);

	// Poll once immediately so the first second is not spent showing "connecting".
	void (async () => {
		await loadPresetSettings();
		applyLiveState(await pollClientApi());
	})();

	/** The clock the guide counts against: the game's own, or the manual fallback. */
	// `displayTime` is null when the client answered without a clock; treat that the same as
	// no game, so the overlay shows its manual controls instead of counting from nothing.
	const clock = $derived(
		usingManual || !live.game || live.game.displayTime === null ? manualClock : live.game.displayTime
	);

	/** How far through the build that clock is. */
	const progress = $derived(advance(build.steps, clock));

	/** Matchup implied by the live players, when the client knows them. */
	const liveMatchup = $derived.by((): Matchup | null => {
		const players = live.game?.players;
		if (!players || players.length < 2) return null;
		return matchupOf(players[0].race, players[1].race);
	});

	/**
	 * Loads the preset build whenever the matchup changes.
	 *
	 * Keyed on the matchup rather than on the poll, so this runs once per game instead of once
	 * per second, and a game that ends and restarts with the same matchup does not reload. The
	 * lookup prefers an exact `ZvP` preset and then a `ZvX` wildcard, mirroring the library.
	 *
	 * It only ever fires on a *change* to a matchup the client reports, so a player who picks
	 * their race randomly is not switched around mid-game: the client reports the settled race,
	 * and a preset is chosen from that once.
	 */
	$effect(() => {
		const matchup = liveMatchup;
		if (!matchup) return;
		const { build: next, fromPreset } = resolvePresetBuild(presets, matchup, sampleBuildOrder);
		if (next.id === build.id) return;
		build = next;
		presetMatchup = fromPreset ? next.matchup : null;
	});

	/**
	 * Loads the configured presets once, on mount.
	 *
	 * Read from settings rather than sent over an event so the overlay works when it is opened
	 * before the main window has ever been shown, which is the normal case at game start.
	 */
	async function loadPresetSettings() {
		try {
			presets = (await loadPresets()).presets;
		} catch {
			// A browser has no settings to read; the sample build stands.
		}
	}

	/**
	 * One honest sentence describing the live connection.
	 *
	 * Indexed by a `Record<LiveStatus, string>` rather than a switch so the compiler proves
	 * exhaustiveness: adding a status without a message becomes a type error, which a
	 * switch-with-default would silently swallow.
	 */
	const statusLabels: Record<LiveState['status'], string> = {
		idle: 'Looking for StarCraft II\u2026',
		connecting: 'Looking for StarCraft II\u2026',
		live: 'Reading the game clock',
		replay: 'A replay is playing in the client',
		unavailable: 'Client API not reachable \u2014 is the game running?',
		error: 'Client API returned something unexpected'
	};

	/** The status line, preferring the transport's own message when there is one. */
	const statusText = $derived(
		live.detail ??
		statusLabels[live.status] +
			(live.game?.displayTime != null ? ` (${live.game.displayTime.toFixed(0)}s)` : '')
	);

	/** In a browser tab, loopback is unreadable, so say that instead of blaming the game. */
	const statusHint = $derived(
		live.status === 'unavailable' && !isTauri()
			? 'Client API needs the desktop app; a browser cannot read loopback.'
			: null
	);

	onDestroy(() => clearInterval(timer));

	// The window itself is transparent; the page has to agree, or the global `body`
	// background would fill the overlay with an opaque rectangle. The class is added to
	// `<html>` only while this route is mounted so every other route keeps its background.
	$effect(() => {
		if (typeof document === 'undefined') return;
		document.documentElement.classList.add('is-transparent-window');
		return () => document.documentElement.classList.remove('is-transparent-window');
	});

	/** Moves the manual clock, used only while the client API is unavailable. */
	function nudge(delta: number) {
		usingManual = true;
		manualClock = Math.max(0, manualClock + delta);
	}

	/**
	 * Space nudges the manual clock, and Tab resets it. Bound on the window because the
	 * overlay is click-through: the player cannot click it, so keyboard is the only reach.
	 *
	 * @param event The keydown event.
	 */
	function onKeydown(event: KeyboardEvent) {
		if (event.key === ' ') {
			event.preventDefault();
			nudge(5);
		} else if (event.key === 'Tab') {
			event.preventDefault();
			usingManual = true;
			manualClock = 0;
		}
	}

	/** 0-based step number rendered next to each row. */
	const stepNumber = (index: number) => index + 1;
</script>

<svelte:window on:keydown={onKeydown} />

<div
	class="flex h-full flex-col text-ink-100"
	style="background-color: color-mix(in oklab, var(--color-surface-950) {panelOpacity}%, transparent)"
>
	<header class="flex items-center justify-between gap-2 border-b border-surface-800 px-3 py-2">
		<div class="min-w-0">
			<h1 class="truncate text-sm font-semibold">{build.title}</h1>
			<p class="tabular text-xs text-ink-500">
				{liveMatchup ?? presetMatchup ?? build.matchup}{#if usingManual} · manual{/if}
			</p>
		</div>
		<div class="flex items-center gap-1">
			<button
				type="button"
				class="rounded px-2 py-1 text-xs text-ink-300 hover:bg-surface-800"
				onclick={() => nudge(-5)}
				aria-label="Rewind the manual clock five seconds"
			>
				&#8722;5s
			</button>
			<button
				type="button"
				class="rounded px-2 py-1 text-xs text-ink-300 hover:bg-surface-800"
				onclick={() => nudge(5)}
				aria-label="Advance the manual clock five seconds"
			>
				+5s
			</button>
		</div>
	</header>

	<p
		class="border-b border-surface-800 px-3 py-1.5 text-[11px] {live.status === 'live'
			? 'text-ahead'
			: 'text-ink-500'}"
		role="status"
	>
		{statusText}
	</p>
	{#if statusHint}
		<p class="border-b border-surface-800 bg-surface-900/60 px-3 py-1.5 text-[11px] text-ink-500">
			{statusHint}
		</p>
	{/if}

	<ol class="min-h-0 flex-1 overflow-y-auto" aria-label="Build order steps">
		{#each build.steps as step, index (index)}
			{@const isNext = index === progress.nextIndex}
			{@const isPast = index <= progress.lastDueIndex}
			{@const isMissed = progress.missedIndexes.includes(index)}
			<li
				class="tabular flex items-center gap-2 px-3 py-1.5 text-sm {isNext
					? 'bg-surface-800 font-semibold text-ink-100'
					: isMissed
						? 'text-behind'
						: isPast
							? 'text-ink-500 line-through'
							: 'text-ink-300'}"
			>
				<span class="w-6 shrink-0 text-right text-xs text-ink-500">{stepNumber(index)}</span>
				<span class="w-9 shrink-0 text-right text-xs text-ink-500">{step.supply}</span>
				<span class="w-11 shrink-0 text-xs">{formatGameTime(step.timeInGameSeconds)}</span>
				<span class="min-w-0 flex-1 truncate">{step.name}</span>
				{#if isMissed}
					<span class="text-xs" title="Past its target time">late</span>
				{/if}
			</li>
		{/each}
	</ol>

	<footer class="border-t border-surface-800 px-3 py-2">
		<p class="tabular text-xs text-ink-500">
			Game clock {formatGameTime(clock)}
			{#if progress.nextIndex >= 0}
				&middot; next at {formatGameTime(build.steps[progress.nextIndex].timeInGameSeconds)}
			{:else}
				&middot; build complete
			{/if}
			{#if progress.missedIndexes.length > 0}
				&middot; <span class="text-behind">{progress.missedIndexes.length} late</span>
			{/if}
		</p>
		<p class="mt-1 text-[11px] leading-tight text-ink-500">
			Fixed steps only — it does not watch either player. Space advances the manual clock.
		</p>
	</footer>
</div>

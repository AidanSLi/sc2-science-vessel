<script lang="ts">
	/**
	 * The build-order guide: what the app's window shows while a game is being played.
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
	 *
	 * This is a component rather than a route because the app has **one window with two
	 * modes** (see `#lib/window-mode`), not two windows. The window's shape changes between
	 * a normal analysis window and this click-through panel; the view is swapped inside it.
	 */
	import { onDestroy } from 'svelte';
	import { advance } from '#lib/builds';
	import { resolvePresetBuild } from '#lib/build-library';
	import { formatGameTime, matchupOf } from '#lib/clock';
	import { isTauri, pollClientApi, type LiveState } from '#lib/client-api';
	import { loadPresets } from '#lib/settings';
	import { setWindowMode } from '#lib/window-mode';
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
		screens: null,
		lastOkAt: null,
		detail: null
	});

	/**
	 * Whether the guide has a clock to count against at all.
	 *
	 * There is deliberately no manual clock. An earlier version had +/- buttons and a space-bar
	 * nudge so the player could correct the reading, but a guide the player can nudge is a guide
	 * that can be told "you are here" without the game having said so, and the whole point of
	 * this view is that it knows nothing except the predetermined list and the game's own clock.
	 * When the clock cannot be read the guide says so and stays at the first step.
	 */
	const hasClock = $derived(!(live.status === 'unavailable' || live.status === 'error') && live.game?.displayTime != null);

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
	 * Stores the latest poll.
	 *
	 * Nothing is derived from it here — `clock` and `hasClock` read `live` directly — because
	 * every extra piece of state that tracked the connection was another thing that could
	 * disagree with it.
	 *
	 * @param state The freshly polled client state.
	 */
	function applyLiveState(state: LiveState) {
		live = state;
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

	/**
	 * The clock the guide counts against: the game's own, and nothing else.
	 *
	 * `displayTime` is null when the client answered without a clock, which is treated the same
	 * as no game — the guide holds at step one rather than counting from zero, because counting
	 * from zero would silently claim the player was early for everything.
	 */
	const clock = $derived(hasClock ? (live.game?.displayTime ?? 0) : 0);

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

	// The window itself is transparent while in overlay mode; the page has to agree, or the
	// global `body` background would fill it with an opaque rectangle. The class goes on
	// `<html>` only while this component is mounted, so the analysis view keeps its background.
	$effect(() => {
		if (typeof document === 'undefined') return;
		document.documentElement.classList.add('is-transparent-window');
		return () => document.documentElement.classList.remove('is-transparent-window');
	});

	/**
	 * Escape leaves the guide.
	 *
	 * Bound on the window because the guide is click-through in this mode: the player cannot
	 * click it, so the keyboard is the only reach. Escape is not a nicety — it is the *only* way
	 * out of the guide while it is click-through, since the rest of the app is unreachable from
	 * here. There are no other keys: the guide takes no input that could move its clock.
	 *
	 * @param event The keydown event.
	 */
	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			// Deliberately does not hide the app: it returns to the analysis view. Hiding is the
			// hotkey's job, and a player escaping the guide usually wants to look at the app, not
			// to make it vanish. The automatic switch will put the guide back if a game is still
			// running, which is the correct outcome — the guide is the right view during a game.
			void setWindowMode('analysis');
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
	<header class="flex items-center justify-between gap-2 guide-rule border-b px-3 py-2">
		<div class="min-w-0">
			<h1 class="truncate text-sm font-semibold">{build.title}</h1>
			<p class="tabular text-xs text-ink-500">
				{liveMatchup ?? presetMatchup ?? build.matchup}
			</p>
		</div>
	</header>

	<p
		class="guide-rule border-b px-3 py-1.5 text-[11px] {live.status === 'live'
			? 'text-ahead'
			: 'text-ink-500'}"
		role="status"
	>
		{statusText}
	</p>
	{#if statusHint}
		<p class="guide-rule guide-veil border-b px-3 py-1.5 text-[11px] text-ink-500">
			{statusHint}
		</p>
	{/if}

	<ol class="min-h-0 flex-1 overflow-y-auto" aria-label="Build order steps">
		{#each build.steps as step, index (index)}
			{@const isNext = index === progress.nextIndex}
			{@const isPast = index <= progress.lastDueIndex}
			{@const isMissed = progress.missedIndexes.includes(index)}
			<li
				class="tabular guide-veil flex items-center gap-2 px-3 py-1.5 text-sm {isNext
					? 'font-semibold text-ink-100'
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
			</li>
		{/each}
	</ol>

	<footer class="guide-rule border-t px-3 py-2">
		<p class="tabular text-xs text-ink-300">
			{#if progress.nextIndex >= 0}
				{@const next = build.steps[progress.nextIndex]}
				Next: {next.supply} &nbsp;|&nbsp; {next.name}
			{:else}
				Build complete
			{/if}
		</p>
	</footer>
</div>

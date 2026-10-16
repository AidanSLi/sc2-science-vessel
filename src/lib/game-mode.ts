/**
 * Decides which of its two roles the app's window should be playing, from what the game
 * client reports.
 *
 * This is the *automatic* half of the window's behaviour: the player presses the hotkey to
 * show or hide the app, and the app decides for itself whether it is currently a build-order
 * guide or an ordinary analysis window.
 *
 * ## The signal is the client's screen list, not the clock
 *
 * The first version of this module inferred "the game has ended" from **the clock standing
 * still**, on the theory that a stopped clock means either a pause or a finished game. That
 * was wrong, and it is worth recording why, because the mistake was invisible from the data
 * it was based on. The clock freezes on a pause *and* at the end of a game, so the two are
 * genuinely indistinguishable from the clock alone — which meant the app could not switch
 * back to the analysis view on a pause without also switching back when the game really
 * ended. It picked a five-second threshold and hoped.
 *
 * The client had the answer all along on the `/ui` endpoint, which had been written off
 * because a single probe during a game returned `{"activeScreens":[]}` and that was taken as
 * "no information". An empty list *was* the information. Confirmed against captured
 * transitions from github.com/Rairden/sc2-scenes and the screen table in
 * github.com/leigholiver/OBS-SC2Switcher:
 *
 * - **Empty** while a game is being played. This is the in-game reading.
 * - **`ScreenScore/ScreenScore`** once the match is over and the player is on the post-game
 *   screen — the exact moment the user asked the app to switch back, and reported as such.
 * - **`ScreenLoading/ScreenLoading`** while a map loads, between the lobby and the game.
 * - Anything else (`ScreenHome`, `ScreenMultiplayer`, `ScreenSingle`, …) means the player is
 *   in the menus.
 *
 * So the mode follows the screen list directly and there is no threshold to tune and no
 * pause/end ambiguity left. `/ui` is read on the same poll as `/game` (see
 * `#lib/client-api`).
 *
 * ## When the client cannot be read
 *
 * `screens` is `null`, which is distinct from empty, and the mode is then left alone rather
 * than guessed. A player whose client is briefly slow to answer must not watch the window
 * change shape underneath them.
 *
 * ## Replays
 *
 * Opening a replay leaves a live game's screen list — empty — so a replay would otherwise
 * show the build-order guide. It is deliberately sent to the analysis view instead: a replay
 * is being *watched*, not played, and the player needs the replay controls to be clickable,
 * which a click-through overlay does not allow. This is a judgement call rather than a
 * derivation, so it is stated here rather than buried in a branch.
 */
import type { LiveState } from './client-api';
import { SCORE_SCREEN, isScreenActive } from './client-api';

/** Which role the window should be playing. */
export type WindowMode = 'analysis' | 'overlay';

/**
 * Folds one client poll into the mode the window should be in.
 *
 * Returns the **target mode**, not a delta. Choosing not to repeat a mode that is already
 * applied is the caller's job, because the caller is the thing that knows which mode the shell
 * actually put the window in — see `#lib/window-mode`, whose store mirrors the shell. A
 * "changed?" flag returned from here would be a second copy of that state and would drift from
 * it the first time a mode were set from anywhere else (the settings page, the shell's own
 * startup affordance).
 *
 * The checks are ordered from most to least specific:
 *
 * 1. **The client answered nothing useful** — which view is right is unknown, so `null`.
 * 2. **A replay** — always the analysis view, for the reason in the module note. This must
 *    precede the screen test, because a replay's screen list is the same empty reading as a
 *    live game.
 * 3. **The screen list is unreadable (`null`)** — `null`, as above.
 * 4. **The score screen is open** — the game is over and the player is on the post-game
 *    screen, so the analysis view. Checked before "any screen open" because it *is* a screen,
 *    and it is the one that means "finished" rather than "pending".
 * 5. **Any other screen is open** — the menus, or loading on the way into a game, so the
 *    analysis view.
 * 6. **No screen is open at all** — the in-game reading, so the build-order guide.
 *
 * @param state The latest polled client state.
 * @returns The mode the window should be in, or `null` when that cannot be determined.
 */
export function decideMode(state: LiveState): WindowMode | null {
	// Nothing usable from the client: say nothing rather than flip on a hiccup.
	if (state.status === 'unavailable' || state.status === 'error') return null;

	// A replay is watched, not played, and needs clickable controls — so analysis, even
	// though its screen list is the same empty reading as a live game.
	if (state.status === 'replay') return 'analysis';

	const { screens } = state;

	// `/ui` did not answer, so the screens in play are genuinely unknown.
	if (screens === null) return null;

	// The post-game score screen: the match is over. This is the transition the user asked for
	// by name, and it is the whole reason the screen list is consulted at all.
	if (isScreenActive(screens, SCORE_SCREEN)) return 'analysis';

	// Any other open screen means the player is in the menus, or loading on the way into a
	// game, which is not somewhere a build order is being followed.
	if (screens.length > 0) return 'analysis';

	// No screen open at all: the in-game reading.
	return 'overlay';
}

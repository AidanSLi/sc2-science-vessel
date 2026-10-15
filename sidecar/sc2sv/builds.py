"""Bridge to `spawningtool`, which supplies the guide-shaped build order.

`spawningtool` is MIT-licensed, actively maintained, and already solves the hard part of build
order extraction: it classifies each step, flags workers, orders by supply, and knows the
per-build tables of what is excluded from a build order. The app does not reimplement that.
See `docs/adr/0001` for why it runs in the sidecar rather than being ported.

**Its clock is wrong for Legacy of the Void, and this module is where that is corrected.**
`spawningtool/lotv_constants.py` hardcodes `FRAMES_PER_SECOND = 22.4`, and its step `time`
strings are formatted with that divisor. 22.4 is `16 x 1.4` — the Wings of Liberty and Heart of
the Swarm "Faster" factor, removed in LotV. The `frame` values themselves are raw engine
frames and are correct; only the label derived from them is wrong.

Verified by joining the two streams on one replay: `spawningtool`'s pylon frames at
`frame / 16` are `[44.44, 168.69, 234.00, 307.75, 456.12, ...]` while the tracker stream's
`UnitInitEvent` seconds for the same player's pylons are `[44, 168, 234, 307, 456, ...]` — a
1:1 match. Dividing by 22.4 instead would place them at `[31.74, 120.49, 167.14, 219.82, ...]`,
which is 1.4x low. So the app must ignore `spawningtool`'s `time` and recompute from `frame`.

The wrong clock matters more than it looks. The whole point of the build-order comparison is
the user's requested "+/- 10 seconds" tolerance; a 1.4x error would put every step 40% off and
report every build as badly mistimed, with no error anywhere.
"""

from __future__ import annotations

import pathlib
from typing import Any

from .replay import GAME_FPS

# Set by `spawningtool` as `16 * 1.4`, the pre-LotV Faster factor. Recorded here so the
# correction is self-documenting and a future reader can see exactly what is being undone.
SPAWNINGTOOL_FRAMES_PER_SECOND = 22.4


def _step_from_spawningtool(raw: dict[str, Any]) -> dict[str, Any]:
	"""Converts one `spawningtool` build step to the app's step shape on the correct clock.

	Args:
	    raw: A step from `players[pid]["buildOrder"]`.

	Returns:
	    A step with `name`, `second` (game clock), `supply`, `isWorker` and `isChronoboosted`.
	    The original `time` string is deliberately dropped rather than carried along: keeping
	    two clocks in one object is how the wrong one gets used.
	"""
	frame = float(raw.get("frame") or 0.0)
	supply = raw.get("supply")
	return {
		"name": str(raw.get("name") or ""),
		"second": round(frame / GAME_FPS, 1),
		"supply": int(supply) if isinstance(supply, (int, float)) else None,
		"isWorker": bool(raw.get("is_worker", False)),
		"isChronoboosted": bool(raw.get("is_chronoboosted", False)),
	}


def parse_build_orders(path: str | pathlib.Path) -> dict[int, dict[str, Any]]:
	"""Extracts every player's build order, on the app's clock.

	The result is keyed by **real player id**, not by the string keys `spawningtool` returns, so
	callers can join it against `PlayerInfo.pid` without a conversion step at every use site.

	Steps that appear twice at the same frame are collapsed. `spawningtool` emits duplicates for
	a single action seen through two event streams — two `Drone` entries at frame 680.2 and two
	`Queen` entries four frames apart were observed in the reference replay — and a duplicate in
	a build order reads as "the player built two of these", which is wrong.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    Per-pid dicts with `name`, `race`, `result`, `isWinner`, `isHuman` and `steps`. Returns
	    an empty dict when `spawningtool` cannot parse the replay, because a caller analysing a
	    replay should still get the tracker-derived metrics rather than nothing.
	"""
	from spawningtool.parser import parse_replay

	parsed = parse_replay(str(path))
	if not parsed:
		return {}

	players: dict[int, dict[str, Any]] = {}
	for pid_text, player in (parsed.get("players") or {}).items():
		try:
			pid = int(pid_text)
		except (TypeError, ValueError):
			continue
		seen: set[tuple[str, float]] = set()
		steps: list[dict[str, Any]] = []
		for raw in player.get("buildOrder") or []:
			step = _step_from_spawningtool(raw)
			if not step["name"]:
				continue
			identity = (step["name"], step["second"])
			if identity in seen:
				continue
			seen.add(identity)
			steps.append(step)
		steps.sort(key=lambda step: step["second"])
		players[pid] = {
			"name": str(player.get("name") or ""),
			"race": str(player.get("race") or ""),
			"result": str(player.get("result") or ""),
			"isWinner": bool(player.get("is_winner", False)),
			"isHuman": bool(player.get("is_human", True)),
			"steps": steps,
		}
	return players


def reference_steps(players: dict[int, dict[str, Any]], pid: int) -> list[dict[str, Any]]:
	"""Returns one player's steps in the shape `compare_to_build` consumes.

	A reference build stored in the app's snapshot has no positions or ownership, so the only
	fields the comparison can rely on are the name, the time and (when known) the supply.

	Args:
	    players: The result of `parse_build_orders`.
	    pid: The player whose steps to return.

	Returns:
	    A build order, or an empty list when the player is unknown.
	"""
	player = players.get(pid)
	if not player:
		return []
	return [
		{"name": step["name"], "second": step["second"], "supply": step.get("supply")}
		for step in player["steps"]
	]

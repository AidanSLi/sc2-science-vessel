"""Regression tests for the ownership model in `sc2sv.identity`.

These are the tests that would have caught the composition bug: every unit count came back
empty because tracker events were read for ownership through `event.player`, which is None on
`UnitBornEvent` and `UnitInitEvent`. The assertions below pin the four ownership conventions
in place so the same mistake cannot return silently — and each one is written so that reading
the *wrong* field produces a specific, non-empty but incorrect answer.

See `docs/adr/0007` for why attribution is centralised in one module.
"""

from __future__ import annotations

from sc2sv.identity import NEUTRAL_CONTROL_PID, PlayerDirectory
from sc2sv.replay import load_header

from .conftest import find_replay


def _directory_for(replay):
	"""Builds a `PlayerDirectory` for a loaded replay.

	Args:
	    replay: A loaded sc2reader replay at any load level.

	Returns:
	    The directory, plus the list of real player ids.
	"""
	header = load_header(str(replay.filename))
	return PlayerDirectory(header.players), [player.pid for player in header.players]


def test_births_are_owned_through_control_pid_not_player():
	"""Births must be attributed at all, and neutral spawns must stay neutral.

	This is the exact failure that shipped: `event.player` is None on `UnitBornEvent`, so a
	`player`-based lookup reports zero births for every player while still looking like valid
	code. The neutral half matters as much — `control_pid == 0` is the map's owner, and treating
	it as a player id would hand 2949 `InvisibleTargetDummy` births to whichever player is first.
	"""
	pytest_names = ("(P)Gemini VS (Z)S", "LiquidTLO vs Thorzain", "patch_5_0_16")
	path = find_replay(*pytest_names)
	if path is None:
		import pytest

		pytest.skip(f"fixture replay not available; looked for {', '.join(pytest_names)}")
	import sc2reader

	replay = sc2reader.load_replay(str(path), load_level=4)
	directory, pids = _directory_for(replay)

	owned: dict[int, int] = {pid: 0 for pid in pids}
	neutral = 0
	born = 0
	for event in replay.tracker_events:
		if type(event).__name__ != "UnitBornEvent":
			continue
		born += 1
		owner = directory.owner_of(event)
		if owner is None:
			neutral += 1
		else:
			owned[owner.pid] += 1

	assert born > 0, "no births at all; the replay did not decode"
	assert neutral > 0, "no neutral spawns found; control_pid==0 is not being treated as neutral"
	assert sum(owned.values()) + neutral == born, "every birth must be attributed or neutral"
	# The critical assertion: a `player`-based lookup yields an empty map, so every player must
	# be non-zero here or the bug is back.
	assert all(count > 0 for count in owned.values()), (
		f"a player got no births at all: {owned}. This is the signature of reading "
		"ownership from `event.player` on tracker events, where it is None."
	)


def test_neutral_control_pid_is_zero_and_never_a_player():
	"""`control_pid == 0` must resolve to no player, not to player index zero."""
	path = find_replay("(P)Gemini VS (Z)S", "LiquidTLO vs Thorzain", "patch_5_0_16")
	if path is None:
		import pytest

		pytest.skip("fixture replay not available")
	import sc2reader

	replay = sc2reader.load_replay(str(path), load_level=4)
	directory, _ = _directory_for(replay)

	seen_neutral = False
	for event in replay.tracker_events:
		if type(event).__name__ not in ("UnitBornEvent", "UnitInitEvent"):
			continue
		if getattr(event, "control_pid", None) != NEUTRAL_CONTROL_PID:
			continue
		seen_neutral = True
		assert directory.owner_of(event) is None, (
			"a neutral spawn was attributed to a player; the first player's id is 1, and "
			"control_pid 0 is the map"
		)
	assert seen_neutral, "no neutral spawns in this replay; the assertion proved nothing"


def test_deaths_are_owned_through_the_unit_object():
	"""Deaths carry ownership only on their `unit`, and must not all be neutral.

	`UnitDiedEvent` has no `player` and no `control_pid`; a lookup that ignores `unit.owner`
	reports every death as neutral, which would make the runby and battle-map features silently
	plot nothing.
	"""
	path = find_replay("(P)Gemini VS (Z)S", "LiquidTLO vs Thorzain", "patch_5_0_16")
	if path is None:
		import pytest

		pytest.skip("fixture replay not available")
	import sc2reader

	replay = sc2reader.load_replay(str(path), load_level=4)
	directory, _ = _directory_for(replay)

	deaths = [event for event in replay.tracker_events if type(event).__name__ == "UnitDiedEvent"]
	assert deaths, "no deaths in this replay; the assertion proved nothing"
	attributed = [event for event in deaths if directory.owner_of(event) is not None]
	assert attributed, (
		"no death could be attributed to a player; `UnitDiedEvent` has no `player` or "
		"`control_pid`, so ownership must come from `event.unit.owner`"
	)


def test_team_zero_is_a_real_team():
	"""A team id of 0 must not be treated as missing.

	The first team has `team_id == 0`, so a truthiness check on it returns the given player's
	own team as the opponent list — that is, it reports "no opponents" for half of all games.
	"""
	path = find_replay("(P)Gemini VS (Z)S", "LiquidTLO vs Thorzain", "patch_5_0_16")
	if path is None:
		import pytest

		pytest.skip("fixture replay not available")
	import sc2reader

	replay = sc2reader.load_replay(str(path), load_level=2)
	directory, pids = _directory_for(replay)
	assert len(pids) >= 2, "need at least two players for this assertion"
	for player in directory.players:
		opponents = directory.opponents(player)
		assert opponents, f"player {player.name} (team {player.team}) reported no opponents"
		assert player not in opponents, "a player was listed as their own opponent"

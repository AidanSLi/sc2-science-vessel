"""Tests for build-order extraction and comparison.

Two independent paths produce a build order and they are used for different things, so both
are pinned here:

* `spawningtool`'s parser, which supplies the guide-shaped list (workers flagged, supply
  ordering) and whose step shape the app's reference builds must match;
* `sc2sv.metrics.build_order_from_events`, which supplies positions and the `isStructure`
  flag that `spawningtool` does not provide.

The comparison logic is tested against synthetic builds as well as real ones, because the
interesting cases — a step arriving early, a step missing entirely, a duplicate that must not
be matched twice — are hard to find on demand in a real replay.
"""

from __future__ import annotations

import pytest
import sc2reader

from sc2sv import metrics
from sc2sv.analyze import MATCH_TOLERANCE_SECONDS, compare_to_build
from sc2sv.identity import PlayerDirectory
from sc2sv.replay import load_header

from .conftest import find_replay


def _steps(*pairs: tuple[str, float]) -> list[dict]:
	"""Builds a step list from `(name, second)` pairs.

	Args:
	    *pairs: Alternating unit name and time.

	Returns:
	    A build order in the shape `compare_to_build` consumes.
	"""
	return [{"name": name, "second": second} for name, second in pairs]


def test_compare_matches_within_tolerance_and_reports_the_signed_delta():
	"""A step inside the tolerance matches, and its delta keeps its sign."""
	actual = _steps(("SupplyDepot", 100.0), ("Barracks", 160.0))
	reference = _steps(("SupplyDepot", 103.0), ("Barracks", 150.0))
	result = compare_to_build(actual, reference)

	assert len(result["matched"]) == 2, f"both steps should match: {result}"
	assert not result["missing"], f"nothing should be missing: {result['missing']}"
	depot = next(entry for entry in result["matched"] if entry["name"] == "SupplyDepot")
	barracks = next(entry for entry in result["matched"] if entry["name"] == "Barracks")
	assert depot["deltaSeconds"] == pytest.approx(-3.0), "a step built early has a negative delta"
	assert barracks["deltaSeconds"] == pytest.approx(10.0), "a late step has a positive delta"
	assert depot["verdict"] == "early", depot
	assert barracks["verdict"] == "late", barracks


def test_compare_reports_a_step_outside_the_tolerance_as_missing():
	"""A step built far too late must not be silently matched.

	This is the whole point of the +/- 10 second tolerance: reporting an 11-second-late barracks
	as "on time" would hide exactly the discrepancy the feature exists to surface.
	"""
	actual = _steps(("Barracks", 200.0))
	reference = _steps(("Barracks", 100.0))
	result = compare_to_build(actual, reference)

	assert not result["matched"], "a 100-second miss must not match"
	assert len(result["missing"]) == 1, f"the step must be reported missing: {result}"
	assert result["missing"][0]["name"] == "Barracks"
	assert len(result["extra"]) == 1, "the actual step is still unaccounted for"


def test_compare_tolerance_boundary_is_inclusive():
	"""A step exactly at the tolerance matches, and one second beyond it does not."""
	reference = _steps(("Pylon", 60.0))
	at_limit = compare_to_build(_steps(("Pylon", 60.0 + MATCH_TOLERANCE_SECONDS)), reference)
	beyond = compare_to_build(_steps(("Pylon", 61.0 + MATCH_TOLERANCE_SECONDS)), reference)
	assert at_limit["matched"], "a step exactly at the tolerance must match"
	assert not beyond["matched"], "a step beyond the tolerance must not match"


def test_compare_does_not_maybe_match_one_actual_step_twice():
	"""Each actual step may satisfy at most one reference step.

	A greedy scan without a used-marker would let a single early `Gateway` satisfy two separate
	reference gateways, turning a real missing building into a phantom on-time one.
	"""
	actual = _steps(("Gateway", 95.0))
	reference = _steps(("Gateway", 93.0), ("Gateway", 100.0))
	result = compare_to_build(actual, reference)

	assert len(result["matched"]) == 1, f"one actual step can only match once: {result['matched']}"
	assert len(result["missing"]) == 1, f"the second gateway must be missing: {result['missing']}"


def test_compare_matches_in_sequence_not_globally():
	"""Matching must respect order, since a build order is a sequence.

	With two supply depots 60 seconds apart and two reference targets 5 seconds apart, the first
	target must claim the first depot. A global nearest-match would swap them and report both as
	roughly on time while the player was actually 55 seconds behind.
	"""
	actual = _steps(("SupplyDepot", 100.0), ("SupplyDepot", 160.0))
	reference = _steps(("SupplyDepot", 100.0), ("SupplyDepot", 105.0))
	result = compare_to_build(actual, reference)

	first = result["matched"][0]
	assert first["actualSecond"] == pytest.approx(100.0), (
		f"the first target must take the first depot: {result['matched']}"
	)
	assert len(result["missing"]) == 1, "the second target is 55s late and cannot match"


def test_compare_reports_every_actual_step_as_extra_or_matched():
	"""No actual step may vanish: it is either matched to a target or reported extra."""
	actual = _steps(("Pylon", 30.0), ("Gateway", 60.0), ("Zealot", 75.0))
	reference = _steps(("Pylon", 30.0), ("Gateway", 60.0))
	result = compare_to_build(actual, reference)

	assert len(result["matched"]) + len(result["extra"]) == len(actual), (
		f"steps went missing from the report: {result}"
	)
	assert [entry["name"] for entry in result["extra"]] == ["Zealot"]


def test_compare_of_identical_builds_reports_no_discrepancy():
	"""A build compared to itself must match completely, with every delta at zero."""
	steps = _steps(("Drone", 12.0), ("Overlord", 15.0), ("SpawningPool", 100.0))
	result = compare_to_build(steps, steps)
	assert len(result["matched"]) == len(steps)
	assert not result["missing"] and not result["extra"]
	assert all(entry["deltaSeconds"] == 0 for entry in result["matched"])
	assert all(entry["verdict"] == "on time" for entry in result["matched"])


def test_compare_handles_a_reference_longer_than_the_game():
	"""A build the player abandoned must report missing steps rather than raise."""
	actual = _steps(("Pylon", 30.0))
	reference = _steps(("Pylon", 30.0), ("Gateway", 60.0), ("CyberneticsCore", 90.0))
	result = compare_to_build(actual, reference)
	assert len(result["matched"]) == 1
	assert len(result["missing"]) == 2
	assert [entry["name"] for entry in result["missing"]] == ["Gateway", "CyberneticsCore"]


def test_compare_accepts_the_tolerance_inclusive_of_supply_metadata():
	"""Supply numbers on reference steps must survive into the missing-step report."""
	actual: list[dict] = []
	reference = [{"name": "Pylon", "second": 60.0, "supply": 14}]
	result = compare_to_build(actual, reference)
	assert result["missing"][0]["supply"] == 14, (
		"the supply a step was expected at is useful context and must not be dropped"
	)


def test_spawningtool_build_orders_are_keyed_by_real_player_id():
	"""The bridge must return real player ids, not `spawningtool`'s string keys.

	`spawningtool` keys players by the string "1"/"2"; every other part of the app keys by
	`PlayerInfo.pid`. Leaving the conversion to each call site is how the two get mixed up.
	"""
	from sc2sv.builds import parse_build_orders
	from sc2sv.replay import load_header

	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")
	builds = parse_build_orders(path)
	header_pids = {player.pid for player in load_header(path).players}

	assert builds, "the bridge returned no build orders"
	assert all(isinstance(pid, int) for pid in builds), f"non-int keys: {list(builds)}"
	assert set(builds) <= header_pids, (
		f"build-order keys {sorted(builds)} are not player ids {sorted(header_pids)}"
	)
	for pid, player in builds.items():
		assert player["steps"], f"player {pid} has an empty build order"
		names = [step["name"] for step in player["steps"]]
		assert all(names), f"player {pid} has an unnamed step"
		seconds = [step["second"] for step in player["steps"]]
		assert seconds == sorted(seconds), f"player {pid} build order is not time-ordered (got {seconds[:20]})"


def test_spawningtool_steps_are_relabelled_onto_the_game_clock():
	"""A step's time must be `frame / 16`, not `spawningtool`'s `frame / 22.4` label.

	`spawningtool` hardcodes `FRAMES_PER_SECOND = 22.4` (16 x 1.4, the pre-LotV Faster factor)
	and formats its `time` strings with it. This pins the correction, because a 1.4x error
	would put every step 40% off and silently break the +/-10 second tolerance.
	"""
	from spawningtool.parser import parse_replay

	from sc2sv.builds import parse_build_orders
	from sc2sv.replay import GAME_FPS

	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")

	raw = parse_replay(str(path))
	assert raw, "spawningtool returned nothing"
	# Guard the premise: if spawningtool ever fixes its constant this test is meaningless, not wrong.
	assert raw["frames_per_second"] == 22.4, (
		f"spawningtool's frames_per_second is now {raw['frames_per_second']}; recheck the clock"
	)

	bridged = parse_build_orders(path)
	for pid_text, player in raw["players"].items():
		pid = int(pid_text)
		raw_steps = player["buildOrder"]
		assert raw_steps, f"pid {pid} has no raw steps to compare"
		assert bridged[pid]["steps"][0]["second"] == pytest.approx(
			raw_steps[0]["frame"] / GAME_FPS, abs=0.1
		)
		# The wrong clock is what the raw label says; prove the two differ, so this is a real fix.
		last = raw_steps[-1]
		assert last["frame"] / 22.4 < last["frame"] / GAME_FPS * 0.8, (
			"the raw label is not 1.4x low; the premise of this correction has changed"
		)


def _nearest_deltas(path, clock_fps: float) -> list[float]:
	"""Measures how far each `spawningtool` step is from the tracker event of the same name.

	Joins the two independent streams per player and unit name, taking each step's nearest
	tracker event. The tracker stream is read through `sc2reader`, which reports `frame / 16`.
	The `clock_fps` argument relabels `spawningtool`'s steps, so passing anything other than 16
	shows what the comparison would look like on the wrong clock — which is the whole point of
	the test below.

	Args:
	    path: Path to a replay both libraries can read.
	    clock_fps: Frames per second to divide `spawningtool`'s frames by.

	Returns:
	    One absolute delta in seconds per matched step.
	"""
	import collections

	from spawningtool.parser import parse_replay

	parsed = parse_replay(str(path))
	replay = sc2reader.load_replay(str(path), load_level=4)
	events: dict[tuple[int, str], list[float]] = collections.defaultdict(list)
	for event in replay.tracker_events:
		if type(event).__name__ not in ("UnitInitEvent", "UnitBornEvent"):
			continue
		pid = getattr(event, "control_pid", None)
		if pid:
			events[(pid, str(event.unit_type_name))].append(round(event.second, 2))

	deltas: list[float] = []
	for pid_text, player in parsed["players"].items():
		pid = int(pid_text)
		for raw in player.get("buildOrder") or []:
			candidates = events.get((pid, str(raw["name"])))
			if not candidates:
				continue
			second = float(raw["frame"]) / clock_fps
			nearest = min(candidates, key=lambda candidate: abs(candidate - second))
			deltas.append(abs(nearest - second))
	return deltas


def test_spawningtool_and_the_tracker_stream_agree_on_the_game_clock():
	"""Steps from the two independent paths must land within seconds of each other.

	This is the strongest available check that the corrected clock is right, and it is written as
	a two-clock A/B so it proves itself rather than asserting a number someone believed once.
	The tracker stream's `second` comes from `sc2reader` (`frame / 16`); `spawningtool`'s raw
	`time` labels come from `frame / 22.4`. Joining the two on each divisor and comparing the
	medians shows which divisor actually lines the streams up.
	"""
	from sc2sv.builds import SPAWNINGTOOL_FRAMES_PER_SECOND
	from sc2sv.replay import GAME_FPS

	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")

	correct = sorted(_nearest_deltas(path, GAME_FPS))
	wrong = sorted(_nearest_deltas(path, SPAWNINGTOOL_FRAMES_PER_SECOND))
	assert len(correct) > 50, f"too few matched steps to judge the clock: {len(correct)}"
	assert len(correct) == len(wrong), "both clocks must be judged on the same steps"

	correct_median = correct[len(correct) // 2]
	wrong_median = wrong[len(wrong) // 2]
	assert correct_median < 15.0, (
		f"median disagreement on the game clock is {correct_median:.1f}s, which is too large for "
		"the two streams to be describing the same events"
	)
	assert wrong_median > correct_median * 5, (
		f"the 22.4 clock scores {wrong_median:.1f}s against the game clock's {correct_median:.1f}s; "
		"if these are comparable the test no longer distinguishes the clocks"
	)


def test_tracker_build_order_records_positions_for_structures():
	"""Structures must carry coordinates, since placement analysis depends on them."""
	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")
	replay = sc2reader.load_replay(str(path), load_level=4)
	header = load_header(str(path))
	steps_by_pid = metrics.build_order_from_events(replay, PlayerDirectory(header.players))

	structures = [
		step
		for steps in steps_by_pid.values()
		for step in steps
		if step["isStructure"]
	]
	assert structures, "no structures found; placement analysis would have nothing to plot"
	with_position = [
		step for step in structures if step["x"] is not None and step["y"] is not None
	]
	assert len(with_position) == len(structures), (
		f"only {len(with_position)}/{len(structures)} structures have coordinates; building "
		"placement needs all of them"
	)


def test_a_modified_reference_build_produces_actionable_discrepancies():
	"""The comparison must produce a usable report for a real build against a shifted one.

	This is the shape of the feature the user asked for: pick the build you were trying to do,
	then see which steps drifted and by how much.
	"""
	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")
	replay = sc2reader.load_replay(str(path), load_level=4)
	header = load_header(str(path))
	steps_by_pid = metrics.build_order_from_events(replay, PlayerDirectory(header.players))

	pid = next(iter(steps_by_pid))
	actual = steps_by_pid[pid]
	assert len(actual) >= 8, "need a build with enough steps to compare"
	# Shift half the steps by 20 seconds: outside the tolerance, so they must be reported.
	reference = [
		{"name": step["name"], "second": step["second"] + (20.0 if index % 2 else 0.0)}
		for index, step in enumerate(actual)
	]
	result = compare_to_build(actual, reference)
	assert result["matched"], "the unshifted half must still match"
	assert result["missing"], "the shifted half is 20s out and must be reported missing"
	for entry in result["matched"]:
		assert abs(entry["deltaSeconds"]) <= MATCH_TOLERANCE_SECONDS, (
			f"a matched step is outside the tolerance: {entry}"
		)


def test_build_similarity_punishes_abandoning_the_plan():
	"""A build that stops early must not score like one that followed the plan badly.

	A ratio of matched-to-*actual* would let a player who built three openers correctly and
	then stopped score 100%; the denominator must be the reference's length.
	"""
	from sc2sv.analyze import build_similarity

	reference = _steps(("Pylon", 30.0), ("Gateway", 60.0), ("CyberneticsCore", 90.0))
	full = compare_to_build(reference, reference)
	partial = compare_to_build(_steps(("Pylon", 30.0)), reference)

	assert build_similarity(full) == pytest.approx(1.0)
	assert build_similarity(partial) == pytest.approx(1 / 3), (
		"abandoning a build after one of three steps is one third done, not one hundred percent"
	)
	assert build_similarity({"matched": [], "missing": [], "extra": []}) == 1.0, (
		"an empty reference asked for nothing, so nothing was missed"
	)


def test_build_similarity_ignores_extra_steps():
	"""Steps the reference never asked for must not deflate the score."""
	from sc2sv.analyze import build_similarity

	reference = _steps(("Pylon", 30.0), ("Gateway", 60.0))
	with_extras = compare_to_build(
		_steps(("Pylon", 30.0), ("SupplyDepot", 40.0), ("Gateway", 60.0), ("Marine", 70.0)),
		reference,
	)
	assert build_similarity(with_extras) == pytest.approx(1.0), (
		f"extra steps should not count against the score: {with_extras}"
	)


def test_closest_build_picks_the_better_matching_candidate():
	"""The nearest-build search must return the candidate the replay actually resembles."""
	from sc2sv.analyze import closest_build

	actual = _steps(("Pylon", 30.0), ("Gateway", 60.0), ("Stargate", 100.0))
	candidates = {
		"2 Gate": _steps(("Pylon", 30.0), ("Gateway", 60.0), ("CyberneticsCore", 95.0)),
		"Stargate opener": _steps(("Pylon", 30.0), ("Gateway", 60.0), ("Stargate", 100.0)),
	}
	best = closest_build(actual, candidates)
	assert best is not None
	assert best["name"] == "Stargate opener", f"wrong candidate chosen: {best}"
	assert best["similarity"] == pytest.approx(1.0)


def test_closest_build_breaks_ties_on_timing_drift():
	"""When two builds match the same number of steps, the tighter one wins.

	`2 Rax` and `3 Rax` share an opener; only the drift distinguishes them, so a tie-break on
	drift is not academic.
	"""
	from sc2sv.analyze import closest_build

	actual = _steps(("SupplyDepot", 25.0), ("Barracks", 50.0))
	candidates = {
		"sloppy": _steps(("SupplyDepot", 15.0), ("Barracks", 45.0)),
		"tight": _steps(("SupplyDepot", 25.0), ("Barracks", 52.0)),
	}
	best = closest_build(actual, candidates)
	assert best is not None
	assert best["name"] == "tight", f"the tie should go to the smaller drift: {best}"


def test_closest_build_returns_none_without_candidates():
	"""An empty candidate set must be reported as "no answer", not as an arbitrary build."""
	from sc2sv.analyze import closest_build

	assert closest_build(_steps(("Pylon", 30.0)), {}) is None


def test_analysis_payload_carries_a_build_order_for_every_player():
	"""`analyse_replay` must expose the guide-shaped build order alongside the tracker data.

	The two are separate views of one game and the UI needs both: the tracker stream gives
	positions and ownership, `spawningtool` gives the ordered, worker-flagged list a player
	would actually follow.
	"""
	from sc2sv.analyze import analyse_replay

	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")
	payload = analyse_replay(path)

	assert payload["players"], "no players in the payload"
	for player in payload["players"]:
		order = player["buildOrder"]
		assert order, f"pid {player['pid']} ({player['name']}) has no build order"
		for step in order:
			assert step["name"], f"unnamed step: {step}"
			assert isinstance(step["second"], float), f"step time is not a float: {step}"
			assert "supply" in step, f"step lost its supply: {step}"
		seconds = [step["second"] for step in order]
		assert seconds == sorted(seconds), f"pid {player['pid']} build order is out of order"
		# The last step must be inside the game, which catches a 1.4x clock error outright.
		assert max(seconds) <= payload["lengthSeconds"] + 1.0, (
			f"pid {player['pid']} has a step at {max(seconds):.0f}s but the game is "
			f"{payload['lengthSeconds']:.0f}s long; the build order is on the wrong clock"
		)

"""Regression tests for the derived metrics in `sc2sv.metrics`.

Every test here runs against a real replay from the local library, because the failure mode
these guard against is not arithmetic — it is "this field means something else in this build".
All three of the bugs found while writing this module were of that kind:

* composition came back empty because births were read for ownership through `event.player`;
* upgrade lists were dominated by `Spray*` and `RewardDance*` entries;
* unit counts were swamped by `InvisibleTargetDummy` and mineral fields.

So the assertions mostly pin *invariants and identities* — a differential must equal
killed minus lost, a play's engagements must be the mirror image of the opponent's, every
death must name a unit — rather than golden numbers, which drift with every balance patch.
"""

from __future__ import annotations

import collections

import pytest
import sc2reader

from sc2sv import metrics
from sc2sv.identity import PlayerDirectory
from sc2sv.replay import load_header

from .conftest import find_replay

# Names that must never appear in any unit metric. They are map objects, not units, and they
# are the reason a naive count reports thousands of "units" in a 30-minute game.
FORBIDDEN_UNIT_FRAGMENTS = ("MineralField", "VespeneGeyser", "InvisibleTargetDummy", "XelNagaTower")


@pytest.fixture(scope="module")
def loaded_replay():
	"""Loads one 1v1 replay once for the whole module.

	Parsing at `load_level=4` takes most of a second per replay, so the module shares one.

	Returns:
	    A loaded sc2reader replay.
	"""
	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		pytest.skip("no 1v1 fixture replay available on this machine")
	return sc2reader.load_replay(str(path), load_level=4)


@pytest.fixture(scope="module")
def directory(loaded_replay) -> PlayerDirectory:
	"""Builds the player directory for the shared replay.

	Args:
	    loaded_replay: The module-scoped replay fixture.

	Returns:
	    A `PlayerDirectory` for that replay.
	"""
	header = load_header(str(loaded_replay.filename))
	return PlayerDirectory(header.players)


def test_build_order_excludes_map_objects(loaded_replay, directory):
	"""No build order may contain a mineral field, geyser or target dummy."""
	steps_by_pid = metrics.build_order_from_events(loaded_replay, directory)
	assert steps_by_pid, "no build steps extracted at all"
	names = {step["name"] for steps in steps_by_pid.values() for step in steps}
	assert names, "build steps have no names"
	offenders = [
		name for name in names if any(fragment in name for fragment in FORBIDDEN_UNIT_FRAGMENTS)
	]
	assert not offenders, f"map objects leaked into the build order: {sorted(offenders)[:6]}"


def test_build_order_marks_structures_and_workers(loaded_replay, directory):
	"""Structures must be distinguishable from units, and workers flagged.

	`UnitInitEvent` covers both buildings and units that are *morphed into existence* — zealots,
	high templars, archons, creep tumors — so without the unit's own `is_building` flag a build
	order reports zealots as buildings and the structure timeline fills with army.
	"""
	steps_by_pid = metrics.build_order_from_events(loaded_replay, directory)
	by_name: dict[str, dict] = {}
	for steps in steps_by_pid.values():
		for step in steps:
			by_name.setdefault(step["name"], step)

	assert by_name, "no steps to inspect"
	# A town hall is unambiguously a structure; a zergling unambiguously is not.
	structures = [name for name, step in by_name.items() if step["isStructure"]]
	units = [name for name, step in by_name.items() if not step["isStructure"]]
	assert structures, "no unit was classified as a structure; is_building is not being read"
	assert units, "no unit was classified as a non-structure; every step is a building"
	workers = [name for name, step in by_name.items() if step["isWorker"]]
	assert workers, "no worker was flagged; the is_worker flag is not being read"
	for name in workers:
		assert not by_name[name]["isStructure"], f"{name} is both a worker and a structure"


def test_every_death_names_a_unit_and_carries_position(loaded_replay, directory):
	"""A death must resolve to a unit name, and must carry coordinates when it can.

	`UnitDiedEvent.unit_type_name` is None in every replay tested, so a missing join shows up
	as a death with an empty name — which then silently drops out of both the runby count and
	the battle map. That is why the assertion is on the name being present at all.
	"""
	deaths = metrics.death_locations(loaded_replay, directory)
	assert deaths, "no deaths extracted"
	unnamed = [death for death in deaths if not death["unit"]]
	assert not unnamed, f"{len(unnamed)} deaths have no unit name; the born/init join is broken"
	offenders = [
		death["unit"]
		for death in deaths
		if any(fragment in death["unit"] for fragment in FORBIDDEN_UNIT_FRAGMENTS)
	]
	assert not offenders, f"map objects appear as deaths: {set(offenders)}"
	positioned = [death for death in deaths if death["x"] is not None and death["y"] is not None]
	# Coordinates are present on 100% of deaths in every replay measured, so this is a real
	# invariant rather than a soft expectation.
	assert len(positioned) == len(deaths), (
		f"only {len(positioned)}/{len(deaths)} deaths have coordinates; the battle map needs "
		"all of them"
	)
	assert any(death["isWorker"] for death in deaths), (
		"no death was flagged as a worker; the runby feature has nothing to count"
	)


def test_deaths_are_attributed_to_a_player(loaded_replay, directory):
	"""Most deaths must belong to a player, not to the neutral map."""
	deaths = metrics.death_locations(loaded_replay, directory)
	attributed = [death for death in deaths if death["ownerPid"] is not None]
	assert attributed, "no death could be attributed to a player"
	assert len(attributed) / len(deaths) > 0.5, (
		f"only {len(attributed)}/{len(deaths)} deaths were attributed; ownership is being read "
		"from a field that is empty on UnitDiedEvent"
	)


def test_composition_counts_real_units_only(loaded_replay, directory):
	"""The composition summary must be non-empty and free of map objects."""
	steps = metrics.build_order_from_events(loaded_replay, directory)
	composition = metrics.army_composition(steps)
	assert composition, "no composition produced"
	for pid, counts in composition.items():
		assert counts, f"player {pid} has an empty composition"
		offenders = [
			name
			for name in counts
			if any(fragment in name for fragment in FORBIDDEN_UNIT_FRAGMENTS)
		]
		assert not offenders, f"map objects in composition for pid {pid}: {offenders}"
		assert sum(counts.values()) > 10, f"player {pid} built only {sum(counts.values())} things"


def test_upgrades_exclude_cosmetics(loaded_replay, directory):
	"""No spray, reward or dance entry may survive into the upgrade list."""
	by_pid = metrics.upgrades(loaded_replay, directory)
	names = [entry["name"] for entries in by_pid.values() for entry in entries]
	assert names, "no upgrades found at all; the probe replay has zerglingmovementspeed"
	for name in names:
		lowered = name.lower()
		for marker in ("spray", "reward", "dance"):
			assert marker not in lowered, f"cosmetic entry survived filtering: {name}"


def test_player_stats_are_sampled_no_faster_than_every_five_seconds(loaded_replay, directory):
	"""`PlayerStatsEvent` sampling cadence is the precision floor for every sampled metric.

	Recorded as a test rather than a comment because the whole design depends on it: nothing
	derived from a stat sample may be reported as more precise than the sample interval.
	"""
	by_pid = metrics.collect_stats(loaded_replay, directory)
	assert by_pid, "no stat samples collected"
	for pid, samples in by_pid.items():
		seconds = [sample.second for sample in samples]
		assert seconds == sorted(seconds), f"samples for pid {pid} are not time-ordered"
		gaps = [b - a for a, b in zip(seconds, seconds[1:])]
		if gaps:
			assert min(gaps) >= 4.9, (
				f"pid {pid} has a {min(gaps):.1f}s gap between stat samples; anything derived "
				"from samples cannot be more precise than the sample interval"
			)


def test_worker_graph_only_reports_drops_as_losses(loaded_replay, directory):
	"""Every reported worker loss must correspond to a real decrease in the curve."""
	samples = metrics.collect_stats(loaded_replay, directory)
	pid = max(samples, key=lambda key: len(samples[key]))
	graph = metrics.worker_graph(samples[pid])
	assert graph["seconds"], "empty worker graph"
	assert len(graph["seconds"]) == len(graph["workers"])
	assert all(count >= 0 for count in graph["workers"]), "negative worker count"
	assert any(count > 0 for count in graph["workers"]), "no workers ever existed"
	for drop in graph["drops"]:
		assert drop["to"] < drop["from"], f"a drop that is not a decrease: {drop}"
		assert drop["lost"] == drop["from"] - drop["to"], f"lost does not equal from-to: {drop}"
		assert drop["sometimeAfter"] <= drop["second"], (
			"a loss must be attributed to an interval, so its start cannot follow its end"
		)


def test_supply_blocks_report_a_bounded_duration(loaded_replay, directory):
	"""A supply block must bound the true duration and start after the game begins."""
	samples = metrics.collect_stats(loaded_replay, directory)
	pid = max(samples, key=lambda key: len(samples[key]))
	blocks = metrics.supply_blocks(samples[pid])
	assert blocks, "no supply blocks found; at least one player in a real game gets capped"
	for block in blocks:
		assert block["minSeconds"] <= block["maxSeconds"], (
			f"the conservative bound exceeds the generous one: {block}"
		)
		assert block["minSeconds"] >= 0, f"negative supply block duration: {block}"
		assert block["sometimeAfter"] >= 0, f"supply block starts before the game: {block}"
		if block["openEnded"]:
			assert block["clearedBy"] is None, "an open-ended block cannot have been cleared"


def test_supply_blocks_never_include_the_opening_samples(loaded_replay, directory):
	"""`food_used == food_made == 0` before anything exists must not count as being capped."""
	samples = metrics.collect_stats(loaded_replay, directory)
	for pid, player_samples in samples.items():
		if not player_samples:
			continue
		first = player_samples[0]
		if first.food_made == 0:
			blocks = metrics.supply_blocks(player_samples)
			assert all(block["sometimeAfter"] > first.second for block in blocks), (
				f"pid {pid} reports a supply block from the very first sample, where supply is 0"
			)


def test_resource_differential_is_killed_minus_lost(loaded_replay, directory):
	"""The differential must equal the client's own cumulative counters, exactly."""
	samples = metrics.collect_stats(loaded_replay, directory)
	pid = max(samples, key=lambda key: len(samples[key]))
	series = metrics.resource_differential(samples[pid])
	assert series["seconds"], "empty differential series"
	assert len(series["seconds"]) == len(series["differential"])
	for sample, value in zip(samples[pid], series["differential"]):
		assert value == sample.resources_killed - sample.resources_lost, (
			"the differential must be the difference of the client's cumulative totals"
		)


def test_engagements_only_cover_intervals_where_something_died(loaded_replay, directory):
	"""An engagement must have a non-zero loss on at least one side."""
	samples = metrics.collect_stats(loaded_replay, directory)
	pid = max(samples, key=lambda key: len(samples[key]))
	engagements = metrics.engagements(samples[pid])
	assert engagements, "no engagements found in a real game"
	for engagement in engagements:
		assert engagement["lost"] or engagement["killed"], (
			f"an engagement with nothing lost or killed: {engagement}"
		)
		assert engagement["net"] == engagement["killed"] - engagement["lost"]
		assert engagement["lost"] >= 0 and engagement["killed"] >= 0, (
			f"a cumulative counter went backwards, which means the sample order is wrong: {engagement}"
		)
		assert engagement["after"] <= engagement["second"]


def test_opponents_engagements_are_mirror_images(loaded_replay, directory):
	"""One player's losses must be the opponent's kills, within the sampling interval.

	This is the identity that proves the loss counters are being read from the right player
	rather than from a fixed slot: if a metric read `players[0]` for everybody, both sides would
	report identical numbers instead of mirrored ones.
	"""
	samples = metrics.collect_stats(loaded_replay, directory)
	if len(samples) < 2:
		pytest.skip("need two players")
	first_pid, second_pid = list(samples)[:2]
	first = metrics.engagements(samples[first_pid])
	second = metrics.engagements(samples[second_pid])
	assert first and second, "both players must have engagements"
	# Totals must be close but need not be identical: the client samples the two players on
	# separate ticks, so a kill can land in different intervals on each side.
	first_total_lost = max(sample.resources_lost for sample in samples[first_pid])
	second_total_killed = max(sample.resources_killed for sample in samples[second_pid])
	assert second_total_killed == pytest.approx(first_total_lost, rel=0.05), (
		f"the opponent's kills ({second_total_killed}) must reconcile with this player's losses "
		f"({first_total_lost}) within the sampling error"
	)


def test_energy_timeline_groups_casts_per_base_and_reports_downtime():
	"""Inject downtime must be measured per hatchery, not across all hatcheries at once.

	Two injects five seconds apart are normal play when they were aimed at two different
	hatcheries and impossible at one, so a cross-base gap measures the player's macro wherever
	noise happens to land. The synthetic case below is exact: two bases, each injected once,
	with one base left idle for 62 seconds.
	"""
	casts = [
		{"second": 100.0, "kind": "inject", "x": 10.0, "y": 10.0},
		{"second": 140.0, "kind": "inject", "x": 200.0, "y": 200.0},
		{"second": 162.0, "kind": "inject", "x": 10.0, "y": 10.0},
	]
	bases = [(10.0, 10.0), (200.0, 200.0)]
	series = metrics.energy_timeline(casts, bases, "inject", 31.7)

	assert series["bases"] == 2, "both hatcheries must be recognised"
	assert series["idealSeconds"] == 31.7
	assert len(series["gaps"]) == 1, f"expected one overlong gap, got {series['gaps']}"
	gap = series["gaps"][0]
	assert gap["from"] == 100.0 and gap["to"] == 162.0, f"wrong gap interval: {gap}"
	assert gap["idleSeconds"] == pytest.approx(30.3, abs=0.05), (
		f"idle time must be the gap minus the ideal cycle: {gap}"
	)
	assert gap["base"] == [10.0, 10.0], "the gap must name the base it happened at"
	assert series["missedSeconds"] == pytest.approx(30.3, abs=0.05)
	# Crucially, the cross-base 40-second gap between t=100 and t=140 is *not* downtime, because
	# those casts were at different bases. A cross-base implementation would report 8.3s here.
	assert all(gap["idleSeconds"] != pytest.approx(8.3, abs=0.05) for gap in series["gaps"])


def test_energy_ignores_negative_gaps_instead_of_clamping_them():
	"""A mis-attributed cast must shrink the sample, not manufacture perfect uptime.

	When two casts land closer together than the energy maths allows, the honest reading is
	that attribution is wrong. Clamping to zero would silently turn that into a flawless macro
	score, so the gap must be dropped.
	"""
	casts = [
		{"second": 10.0, "kind": "inject", "x": 5.0, "y": 5.0},
		{"second": 12.0, "kind": "inject", "x": 5.0, "y": 5.0},
	]
	series = metrics.energy_timeline(casts, [(5.0, 5.0)], "inject", 31.7)
	assert series["gaps"] == [], "an impossible 2s gap must not become a reported gap"
	assert series["missedSeconds"] == 0.0


def test_energy_ideal_constants_match_the_energy_derivation():
	"""The ideal cycle times must come from cost divided by the energy rate.

	Hardcoding a tooltip duration would encode the spell's *effect* length (29s for an inject)
	rather than the binding constraint, which is the 25 energy it costs. This assertion exists
	so a future edit cannot quietly swap one for the other.
	"""
	assert metrics.ENERGY_PER_SECOND == pytest.approx(0.7875), "the Faster-speed energy rate"
	assert metrics.INJECT_IDEAL_SECONDS == pytest.approx(31.7, abs=0.1)
	assert metrics.MULE_IDEAL_SECONDS == pytest.approx(63.5, abs=0.1)
	assert metrics.CHRONO_IDEAL_SECONDS == pytest.approx(63.5, abs=0.1)
	assert metrics.INJECT_IDEAL_SECONDS != pytest.approx(29.0), (
		"29s is the incubation duration; the constraint is the 25 energy that funds it"
	)


def test_ability_casts_are_attributed_and_time_ordered(loaded_replay, directory):
	"""Every energy cast must name a player, and the stream must be time-ordered."""
	casts = metrics.ability_casts(loaded_replay, directory)
	if not casts:
		pytest.skip("this replay contains no inject, MULE or chrono casts")
	seconds = [cast["second"] for cast in casts]
	assert seconds == sorted(seconds), "casts are not time-ordered"
	attributed = [cast for cast in casts if cast["playerPid"] is not None]
	assert len(attributed) == len(casts), (
		f"{len(casts) - len(attributed)} casts have no owner; they cannot be grouped by base"
	)
	kinds = collections.Counter(cast["kind"] for cast in casts)
	assert set(kinds) <= {"inject", "mule", "chrono"}, f"unexpected cast kinds: {kinds}"


def test_town_hall_positions_are_found_for_the_race_that_has_them(loaded_replay, directory):
	"""Hatcheries must be located, since inject attribution depends on it."""
	positions = metrics.town_hall_positions(loaded_replay, directory)
	header = load_header(str(loaded_replay.filename))
	zerg = next((player for player in header.players if player.race == "Zerg"), None)
	if zerg is None:
		pytest.skip("this replay has no Zerg, so it has no hatcheries")
	assert positions.get(zerg.pid), (
		f"no town halls found for the Zerg (pid {zerg.pid}); injects cannot be grouped by base"
	)
	for pid, entries in positions.items():
		assert len(entries) == len(set(entries)), f"duplicate town halls recorded for pid {pid}"


def test_town_hall_positions_are_deduplicated(loaded_replay, directory):
	"""A hatchery reported by both birth and init must be recorded once."""
	positions = metrics.town_hall_positions(loaded_replay, directory)
	for pid, entries in positions.items():
		assert entries == sorted(entries), f"positions for pid {pid} are not sorted"
		assert len(entries) == len(set(entries)), f"duplicates for pid {pid}: {entries}"

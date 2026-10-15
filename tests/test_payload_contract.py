"""Pins the shape of the analysis payload against the frontend's TypeScript types.

The Rust command `analyze_replay` hands the sidecar's JSON straight to the frontend, so the
sidecar's payload and `src/lib/analysis.ts` are two hand-written declarations of one contract
with nothing enforcing agreement. When they drift, the failure is a chart that renders blank
or an `undefined` label — silent, and easy to misread as missing replay data.

This is a snapshot of the fields the frontend reads. It is intentionally a *list of keys*, not
a copy of the values, so changing a computed number does not fail the test while removing a
field does.
"""

from __future__ import annotations

from sc2sv.analyze import analyse_replay

from .conftest import find_replay

# Keys `src/lib/analysis.ts` declares, mirroring `AnalysisPayload` and `AnalysisPlayer`.
TOP_LEVEL_KEYS = {
	"path",
	"build",
	"release",
	"gameType",
	"map",
	"speed",
	"matchup",
	"date",
	"isLadder",
	"lengthSeconds",
	"mmr",
	"players",
	"deaths",
	"workerDeaths",
	"energy",
}

PLAYER_KEYS = {
	"pid",
	"name",
	"race",
	"team",
	"result",
	"isHuman",
	"workers",
	"supplyBlocks",
	"resources",
	"engagements",
	"composition",
	"upgrades",
	"tech",
	"expansions",
	"resourceStructures",
	"firstStructures",
	"buildOrder",
}

DEATH_KEYS = {"second", "unit", "x", "y", "ownerPid", "ownerRace", "isWorker"}

# The energy payload is a per-kind summary, not a cast list. An earlier version of
# `analysis.ts` declared a `casts` array that the sidecar never sent; this set is what caught it.
ENERGY_KEYS = {"kind", "idealSeconds", "bases", "seconds", "gaps", "missedSeconds"}
GAP_KEYS = {"base", "from", "to", "idleSeconds"}

# A tracked step is used for `tech`, `expansions`, `resourceStructures` and `firstStructures` —
# four lists of one record shape.
TRACKED_STEP_KEYS = {"second", "name", "isWorker", "isStructure", "x", "y"}

SUPPLY_BLOCK_KEYS = {"sometimeAfter", "clearedBy", "minSeconds", "maxSeconds", "openEnded"}
ENGAGEMENT_KEYS = {"second", "after", "lost", "killed", "net"}

# Every player list whose entries `analysis.ts` types as `TrackedStep`.
TRACKED_STEP_LISTS = ("tech", "expansions", "resourceStructures", "firstStructures")


def _payload():
	"""Analyses the reference replay, or skips when no fixture is present on this machine."""
	path = find_replay("LiquidTLO vs Thorzain", "patch_5_0_16", "Gemini VS")
	if path is None:
		import pytest

		pytest.skip("no 1v1 fixture replay available on this machine")
	return analyse_replay(path)


def test_payload_top_level_keys_match_the_frontend_type():
	"""Every key the frontend declares must be present, and nothing extra may appear."""
	payload = _payload()
	assert set(payload) == TOP_LEVEL_KEYS, (
		f"missing from payload: {TOP_LEVEL_KEYS - set(payload)}; "
		f"undeclared in analysis.ts: {set(payload) - TOP_LEVEL_KEYS}"
	)


def test_payload_player_keys_match_the_frontend_type():
	"""A player object must carry exactly the fields `AnalysisPlayer` declares."""
	payload = _payload()
	assert payload["players"], "no players in the payload"
	for player in payload["players"]:
		assert set(player) == PLAYER_KEYS, (
			f"pid {player.get('pid')}: missing {PLAYER_KEYS - set(player)}, "
			f"undeclared {set(player) - PLAYER_KEYS}"
		)


def test_death_records_match_the_frontend_type():
	"""Deaths are plotted on the map, so their keys are load-bearing for that view."""
	payload = _payload()
	assert payload["deaths"], "no deaths recorded; the map view would have nothing to draw"
	for death in payload["deaths"]:
		assert set(death) == DEATH_KEYS, f"death shape drifted: {sorted(death)}"


def test_energy_timelines_match_the_frontend_type():
	"""All three energy kinds must be present even when a game has no player of that race.

	An absent key would make the UI's `payload.energy.inject` throw for a ZvZ replay rather
	than report zero, so the keys are unconditional.
	"""
	payload = _payload()
	assert set(payload["energy"]) == {"inject", "mule", "chrono"}
	for kind, timeline in payload["energy"].items():
		assert set(timeline) == ENERGY_KEYS, f"{kind}: {sorted(timeline)}"
		for gap in timeline["gaps"]:
			assert set(gap) == GAP_KEYS, f"{kind} gap shape drifted: {sorted(gap)}"
		for second in timeline["seconds"]:
			assert isinstance(second, (int, float)), f"{kind}: non-numeric cast time {second!r}"


def test_tracked_step_lists_share_one_record_shape():
	"""The four timeline lists must each hold records `TrackedStep` describes.

	They are produced by filtering one stream, so a field added to only one of them would make
	that timeline render differently from its siblings for no visible reason.
	"""
	payload = _payload()
	seen = 0
	for player in payload["players"]:
		for field in TRACKED_STEP_LISTS:
			for step in player[field]:
				assert set(step) == TRACKED_STEP_KEYS, (
					f"pid {player['pid']} {field}: {sorted(step)} != {sorted(TRACKED_STEP_KEYS)}"
				)
				seen += 1
	assert seen, "no tracked steps in any timeline; the fixture is too short to be useful"


def test_supply_blocks_and_engagements_match_the_frontend_types():
	"""Supply blocks are intervals and engagements are trades; the UI reads both field by field."""
	payload = _payload()
	blocks = 0
	for player in payload["players"]:
		for block in player["supplyBlocks"]:
			assert set(block) == SUPPLY_BLOCK_KEYS, f"supply block drifted: {sorted(block)}"
			assert block["minSeconds"] <= block["maxSeconds"], (
				"a block must not claim more certain time than uncertain time"
			)
			blocks += 1
		for engagement in player["engagements"]:
			assert set(engagement) == ENGAGEMENT_KEYS, f"engagement drifted: {sorted(engagement)}"
			assert engagement["net"] == engagement["killed"] - engagement["lost"], (
				f"net must be derived, not stored independently: {engagement}"
			)
	assert blocks, "no supply blocks found; the fixture cannot exercise this feature"


def test_worker_series_are_parallel_arrays():
	"""A chart needs `seconds[i]` and `workers[i]` to be the same point, so lengths must match."""
	payload = _payload()
	for player in payload["players"]:
		workers = player["workers"]
		assert len(workers["seconds"]) == len(workers["workers"]), (
			f"pid {player['pid']}: {len(workers['seconds'])} timestamps but "
			f"{len(workers['workers'])} values; the series is unplottable"
		)
		assert set(workers) == {"seconds", "workers", "drops"}, f"worker keys: {sorted(workers)}"
		resources = player["resources"]
		assert len(resources["seconds"]) == len(resources["differential"]), (
			f"pid {player['pid']}: resource series lengths differ"
		)
		assert set(resources) == {"seconds", "differential"}, f"resource keys: {sorted(resources)}"


def test_worker_drops_are_intervals_within_the_game():
	"""A worker drop is only meaningful as an interval; a precise second would be fabricated."""
	payload = _payload()
	length = payload["lengthSeconds"]
	for player in payload["players"]:
		for drop in player["workers"]["drops"]:
			assert set(drop) == {"second", "from", "to", "lost", "sometimeAfter"}, (
				f"drop shape drifted: {sorted(drop)}"
			)
			assert drop["sometimeAfter"] <= drop["second"], f"drop interval inverted: {drop}"
			assert drop["second"] <= length + 1.0, f"drop past the end of the game: {drop}"
			assert drop["lost"] == drop["from"] - drop["to"], f"drop arithmetic wrong: {drop}"
			assert drop["lost"] > 0, f"a drop must lose workers: {drop}"


def test_every_step_time_is_inside_the_game():
	"""No series may contain a time past the end of the game.

	This is the assertion that catches a clock regression, because a 1.4x error on frames
	pushes late-game steps well beyond the replay's own length.
	"""
	payload = _payload()
	length = payload["lengthSeconds"]
	assert length > 0
	for player in payload["players"]:
		for step in player["buildOrder"]:
			assert 0 <= step["second"] <= length + 1.0, (
				f"pid {player['pid']}: build step {step['name']} at {step['second']}s "
				f"exceeds the game's {length}s"
			)
		for entry in player["upgrades"]:
			assert 0 <= entry["second"] <= length + 1.0, f"upgrade outside the game: {entry}"


def test_payload_is_json_serialisable():
	"""The payload crosses a process boundary as JSON, so nothing exotic may leak into it."""
	import json

	payload = _payload()
	encoded = json.dumps(payload)
	assert encoded, "payload did not serialise"
	assert json.loads(encoded)["players"], "payload did not survive a round trip"


def test_the_payload_reports_the_replay_it_was_asked_about():
	"""The `path` must identify the same file that was analysed.

	Caching keys off this field, so a mismatch would serve one replay's metrics for another.
	"""
	import pathlib

	payload = _payload()
	assert pathlib.Path(payload["path"]).name
	assert payload["map"], "the map name is missing"
	assert payload["date"], "the replay date is missing"

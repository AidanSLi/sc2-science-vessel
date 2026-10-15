"""Assembles one replay's full analysis payload.

This is the sidecar's public entrance: `analyse_replay(path)` returns the single JSON object
the app's /analyze/[id] view renders. Keeping assembly in one place means the CLI and the
Tauri command return byte-identical structures, so a metric is never "working in the CLI but
different in the app".

The payload is deliberately bulky in one dimension and stingy in another: it includes every
prepared series the UI needs, but no raw events. A 40-minute game is ~40,000 events and a few
hundred sample points; only the latter go over the wire.
"""

from __future__ import annotations

import collections
import pathlib
from typing import Any

from . import builds, metrics
from .identity import PlayerDirectory
from .replay import load_events, load_header

# The user asked for build-order discrepancies to be reported at +/- 10 seconds, which is the
# tolerance used when matching a replay's actual build against a reference build.
MATCH_TOLERANCE_SECONDS = 10.0

# Structures whose construction time is worth reporting as a timeline in its own right.
TECH_STRUCTURES = frozenset(
	{
		"Forge",
		"EvolutionChamber",
		"EngineeringBay",
		"CyberneticsCore",
		"TwilightCouncil",
		"RoboticsFacility",
		"Stargate",
		"Spire",
		"BanelingNest",
		"HydraliskDen",
		"InfestationPit",
		"GreaterSpire",
		"Factory",
		"Starport",
		"Armory",
		"GhostAcademy",
		"FusionCore",
		"DarkShrine",
		"TemplarArchives",
		"RoboticsBay",
		"FleetBeacon",
		"NydusNetwork",
		"UltraliskCavern",
		"ShieldBattery",
		"PhotonCannon",
		"MissileTurret",
		"SpineCrawler",
		"SporeCrawler",
		"Bunker",
	},
)

# Structures that mark an expansion rather than a tech choice.
EXPANSION_STRUCTURES = frozenset({"Hatchery", "Lair", "Hive", "CommandCenter", "OrbitalCommand", "PlanetaryFortress", "Nexus"})

# Resource buildings, which are neither tech nor expansion but are worth a timeline.
RESOURCE_STRUCTURES = frozenset({"Refinery", "Extractor", "Assimilator"})


def _pid_of_race(players: list[Any], race: str) -> int:
	"""Finds the player id of the player fielding a given race, or 0 when absent.

	Energy casts belong to a race rather than to a slot: only a Zerg has injects, only a Terran
	has MULEs, only a Protoss has chrono. Returning 0 for "no such player" is safe because the
	lookup that consumes it finds nothing and reports zero casts rather than the wrong player's.

	Args:
	    players: The replay's players.
	    race: The race to find.

	Returns:
	    The matching player's pid, or 0.
	"""
	for player in players:
		if str(getattr(player, "race", "")).lower() == race.lower():
			return int(getattr(player, "pid", 0) or 0)
	return 0


def _structure_timeline(
	steps: list[dict[str, Any]], wanted: frozenset[str]
) -> list[dict[str, Any]]:
	"""Filters a build order down to the structures of interest.

	Args:
	    steps: One player's build steps, time-ordered.
	    wanted: The structure names to keep.

	Returns:
	    The matching steps, unchanged in order.
	"""
	return [step for step in steps if step["name"] in wanted]


def compare_to_build(
	actual: list[dict[str, Any]],
	reference: list[dict[str, Any]],
	tolerance: float = MATCH_TOLERANCE_SECONDS,
) -> dict[str, Any]:
	"""Compares a replay's build order against a reference build order.

	Matching is greedy and in order: each reference step claims the earliest unmatched actual
	step with the same name whose time is within the tolerance. That is the right behaviour for
	this feature because a build order is a sequence — matching a `Gateway` at 3:00 against a
	reference `Gateway` at 3:00 when an earlier `Gateway` was already consumed would report a
	false hit.

	The user asked for discrepancies of +/- 10 seconds to be surfaced, so the tolerance is a
	parameter rather than a constant hidden inside the loop, and every discrepancy reports its
	signed delta so "3 seconds early" and "9 seconds late" stay distinguishable.

	Args:
	    actual: The replay's steps for one player, time-ordered.
	    reference: The reference build's steps, time-ordered.
	    tolerance: Seconds within which a step counts as hitting its target.

	Returns:
	    A dict with `matched`, `discrepancies`, `missing` and `extra` entries.
	"""
	used = [False] * len(actual)
	matched: list[dict[str, Any]] = []
	missing: list[dict[str, Any]] = []
	for target in reference:
		claim = None
		for index, step in enumerate(actual):
			if used[index] or step["name"] != target["name"]:
				continue
			if abs(step["second"] - target["second"]) <= tolerance:
				claim = index
				break
		if claim is None:
			missing.append(
				{"name": target["name"], "expectedSecond": target["second"], "supply": target.get("supply")}
			)
			continue
		used[claim] = True
		delta = round(actual[claim]["second"] - target["second"], 1)
		matched.append(
			{
				"name": target["name"],
				"expectedSecond": target["second"],
				"actualSecond": actual[claim]["second"],
				"deltaSeconds": delta,
				"verdict": "on time" if abs(delta) <= 2.0 else ("early" if delta < 0 else "late"),
			}
		)
	extra = [
		{"name": step["name"], "second": step["second"]}
		for index, step in enumerate(actual)
		if not used[index]
	]
	return {"matched": matched, "missing": missing, "extra": extra}


def build_similarity(result: dict[str, Any]) -> float:
	"""Scores how closely an actual build followed a reference, in the range 0..1.

	The scoring is deliberately asymmetric about what it forgives. A build that stopped early
	should not score like one that followed the plan badly: `matched` counts steps actually hit
	within tolerance and the denominator counts every reference step, so abandoning the plan
	after the opener roughly halves the score rather than being laundered into a high ratio by
	the numerator's own smallness. Steps the player took that the reference never asked for do
	not deflate it either, because `extra` is deliberately absent from the score — building extra
	supply depots is not the same failure as missing a tech structure, and the app reports the
	two separately.

	Args:
	    result: The return value of `compare_to_build`.

	Returns:
	    `matched / reference steps`, or 1.0 when the reference is empty (nothing was asked for,
	    so nothing was missed).
	"""
	reference_count = len(result["matched"]) + len(result["missing"])
	if reference_count == 0:
		return 1.0
	return len(result["matched"]) / reference_count


def closest_build(
	actual: list[dict[str, Any]],
	candidates: dict[str, list[dict[str, Any]]],
	tolerance: float = MATCH_TOLERANCE_SECONDS,
) -> dict[str, Any] | None:
	"""Finds which of several known builds the player was most likely attempting.

	This is the alternative the user asked for to picking a build by hand: instead of naming the
	build after the game, compare against a set and show the nearest. Ties are broken by the
	smaller total timing error, because two builds can match the same number of steps while one
	was followed much more tightly — comparing `2 Rax` against `3 Rax` often matches the same
	opener, and the tighter one is the more useful answer.

	Args:
	    actual: The player's build order from the replay.
	    candidates: Named reference builds to compare against.
	    tolerance: Matching tolerance in seconds.

	Returns:
	    `{name, similarity, totalDriftSeconds, result}` for the best candidate, or None when
	    there are no candidates.
	"""
	best: dict[str, Any] | None = None
	for name, reference in candidates.items():
		result = compare_to_build(actual, reference, tolerance)
		similarity = build_similarity(result)
		drift = sum(abs(entry["deltaSeconds"]) for entry in result["matched"])
		if best is None or (similarity, -drift) > (best["similarity"], -best["totalDriftSeconds"]):
			best = {
				"name": name,
				"similarity": round(similarity, 4),
				"totalDriftSeconds": round(drift, 1),
				"result": result,
			}
	return best


def analyse_replay(path: str | pathlib.Path) -> dict[str, Any]:
	"""Produces the complete prepared analysis payload for one replay.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    The payload: header facts, per-player series, and the shared timelines. Raises if the
	    replay cannot be parsed at all, because a caller asking for one replay's analysis needs
	    to hear about failure rather than receive an empty payload.
	"""
	header = load_header(path)
	if header.error is not None:
		raise ValueError(f"cannot parse replay {path}: {header.error}")

	replay = load_events(path)
	directory = PlayerDirectory(header.players)

	samples_by_pid = metrics.collect_stats(replay, directory)
	births_by_pid = metrics.build_order_from_events(replay, directory)
	upgrades_by_pid = metrics.upgrades(replay, directory)
	deaths = metrics.death_locations(replay, directory)
	casts = metrics.ability_casts(replay, directory)
	composition = metrics.army_composition(births_by_pid)

	casts_by_kind: dict[str, list[dict[str, Any]]] = collections.defaultdict(list)
	for cast in casts:
		casts_by_kind[cast["kind"]].append(cast)
	town_halls = metrics.town_hall_positions(replay, directory)

	# `spawningtool` supplies the guide-shaped build order — the list a player would follow,
	# with workers flagged and steps ordered — while the tracker stream above supplies positions
	# and ownership. Both are needed and they are not interchangeable, so both go in the payload.
	# This is the slowest step in the analysis (it re-parses the replay), so it is allowed to
	# fail: a replay whose build order cannot be extracted still yields every tracker metric.
	build_orders = builds.parse_build_orders(path)

	players: list[dict[str, Any]] = []
	for player in header.players:
		samples = samples_by_pid.get(player.pid, [])
		steps = births_by_pid.get(player.pid, [])
		players.append(
			{
				"pid": player.pid,
				"name": player.name,
				"race": player.race,
				"team": player.team,
				"result": player.result,
				"isHuman": player.is_human,
				"workers": metrics.worker_graph(samples),
				"supplyBlocks": metrics.supply_blocks(samples),
				"resources": metrics.resource_differential(samples),
				"engagements": metrics.engagements(samples),
				"composition": composition.get(player.pid, {}),
				"upgrades": upgrades_by_pid.get(player.pid, []),
				"tech": _structure_timeline(steps, TECH_STRUCTURES),
				"expansions": _structure_timeline(steps, EXPANSION_STRUCTURES),
				"resourceStructures": _structure_timeline(steps, RESOURCE_STRUCTURES),
				"firstStructures": steps[:12],
				"buildOrder": builds.reference_steps(build_orders, player.pid),
			}
		)

	# Deaths are shared rather than per-player: the map view plots both sides at once, and the
	# runby view needs the opponent's worker kills, which belong to the other player's events.
	worker_deaths = [death for death in deaths if death["isWorker"]]

	return {
		"path": header.path,
		"build": header.build,
		"release": header.release,
		"gameType": header.game_type,
		"map": header.map_name,
		"speed": header.speed,
		"matchup": header.matchup,
		"date": header.date,
		"isLadder": header.is_ladder,
		"lengthSeconds": round(header.length_seconds, 1),
		"mmr": header.mmr,
		"players": players,
		"deaths": deaths,
		"workerDeaths": worker_deaths,
		"energy": {
			# Each energy kind belongs to one race's town hall, so the bases are resolved from
			# the race that fields it rather than assumed to be player 1 or 2.
			"inject": metrics.energy_timeline(
				casts_by_kind.get("inject", []),
				town_halls.get(_pid_of_race(header.players, "Zerg"), []),
				"inject",
				metrics.INJECT_IDEAL_SECONDS,
			),
			"mule": metrics.energy_timeline(
				casts_by_kind.get("mule", []),
				town_halls.get(_pid_of_race(header.players, "Terran"), []),
				"mule",
				metrics.MULE_IDEAL_SECONDS,
			),
			"chrono": metrics.energy_timeline(
				casts_by_kind.get("chrono", []),
				town_halls.get(_pid_of_race(header.players, "Protoss"), []),
				"chrono",
				metrics.CHRONO_IDEAL_SECONDS,
			),
		},
	}

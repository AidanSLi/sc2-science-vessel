"""Derived metrics for one replay, as prepared series ready for JSON.

Every function here returns plain dicts and lists of numbers, never sc2reader events. That is
the ADR 0001 contract: the sidecar prepares data, the frontend draws it. A real game produces
tens of thousands of events, and none of them should cross the process boundary.

The metrics come from two replay streams with different characters:

* `PlayerStatsEvent` (tracker) is a **sample**, taken every 10 game seconds. It carries the
  economy and the loss counters. Anything derived from it is precise to one sample, so unit
  counts and worker curves are exact at sample points and interpolated between them.
* `UnitBornEvent` / `UnitDiedEvent` / `UpgradeCompleteEvent` / ability events are **exact
  events** with a frame and often a position. Anything derived from those is exact.

That split decides which features can be trusted to the second, and is why, for example,
supply block duration is reported as a range rather than a number: it is inferred from a
sampled `food_used == food_made` state whose true start lies somewhere inside the gap.
"""

from __future__ import annotations

import collections
from dataclasses import dataclass
from typing import Any

from .identity import PlayerDirectory
from .replay import is_cosmetic_upgrade

# Ability identifiers that carry the energy features. There is no `Inject` or `MULE` name;
# these are what the client actually records, verified present in 11 different replay builds.
INJECT_ABILITY = "SpawnLarva"
MULE_ABILITY = "CalldownMULE"
CHRONO_ABILITY = "ChronoBoostEnergyCost"
# Town-hall unit names, used to locate the base an energy cast was aimed at.
TOWN_HALL_NAMES = frozenset(
	{"Hatchery", "Lair", "Hive", "CommandCenter", "OrbitalCommand", "PlanetaryFortress", "Nexus"}
)

# Energy regeneration at Faster speed, on the game clock. The game clock runs 1:1 with real
# time at Faster, so this is energy per second of the same clock every `second` value uses.
# Liquipedia: "Energy is passively generated at 0.7875 energy per second on Faster speed".
ENERGY_PER_SECOND = 0.7875

# The ideal interval between two casts of one energy ability at **one** base, derived as the
# energy cost divided by the regeneration rate. This is the quantity "missed energy" is
# measured against, so a wrong value biases every number the feature reports.
#
# The derivation is used rather than a tooltip figure because the binding constraint is energy,
# not the spell's duration:
#   * inject costs 25 and incubates for 29 s, but 25 energy needs 31.7 s, so 31.7 is the
#     earliest a queen can re-inject. Standard play cites 29-32 s, which brackets it.
#   * MULE costs 50 and lasts 64 s; 50 energy needs 63.5 s, and Liquipedia states "slightly
#     under 64 seconds for an orbital command to generate 50 energy" - an independent match.
#   * chrono costs 50, giving 63.5 s per nexus. Liquipedia's older Chrono Boost Math page
#     quotes 25 energy per 45 s instead; the 50 figure comes from the spell's own page and is
#     used here, but this is the least certain of the three.
INJECT_COST = 25.0
MULE_COST = 50.0
CHRONO_COST = 50.0
INJECT_IDEAL_SECONDS = INJECT_COST / ENERGY_PER_SECOND
MULE_IDEAL_SECONDS = MULE_COST / ENERGY_PER_SECOND
CHRONO_IDEAL_SECONDS = CHRONO_COST / ENERGY_PER_SECOND

# Marker names for neutral map objects, kept as a second line of defence behind the unit's own
# `race` attribute. Counting these destroys any unit-composition summary: one probe game had
# 2949 `InvisibleTargetDummy` births and only a few hundred real units.
NEUTRAL_UNIT_MARKERS = (
	"mineralfield",
	"vespenegeyser",
	"invisibletargetdummy",
	"xelnagatower",
	"spaceplatformgeyser",
	"destructiblerocks",
)

# The race string sc2reader gives every neutral map object, verified across 5371 neutral
# spawns in 11 builds. This is the primary neutral test because it needs no name list.
NEUTRAL_RACE = "Neutral"


def is_neutral_unit(unit: Any, unit_type: str = "") -> bool:
	"""Reports whether a spawned unit is map scenery rather than something a player made.

	Prefers the unit's own `race` attribute, which is `Neutral` for every map object tested,
	and falls back to a name marker list only when no unit object is available.

	Args:
	    unit: The `Unit` attached to a born/init/died event, or None.
	    unit_type: The unit type name, used when `unit` is None.

	Returns:
	    True when the spawn should be excluded from every unit metric.
	"""
	race = getattr(unit, "race", None) if unit is not None else None
	if race is not None:
		return str(race) == NEUTRAL_RACE
	lowered = unit_type.lower()
	return any(marker in lowered for marker in NEUTRAL_UNIT_MARKERS)


def is_worker_unit(unit: Any, unit_type: str = "") -> bool:
	"""Reports whether a spawned or dying unit is a worker.

	Uses the unit's own `is_worker` flag, which sc2reader sets correctly for SCV, Drone, Probe
	and MULE and leaves false for Larva — a distinction a name list would have to guess at.

	Args:
	    unit: The `Unit` attached to an event, or None.
	    unit_type: The unit type name, used when `unit` is None.

	Returns:
	    True when the unit is a worker.
	"""
	if unit is not None:
		return bool(getattr(unit, "is_worker", False))
	return unit_type.lower() in ("scv", "drone", "probe", "mule")


def _pid_of(event: Any, directory: PlayerDirectory) -> int | None:
	"""Resolves an event's owning player id, or None for neutral events.

	Args:
	    event: Any sc2reader event.
	    directory: The replay's player directory.

	Returns:
	    The player id, or None.
	"""
	player = directory.owner_of(event)
	return player.pid if player is not None else None


@dataclass
class StatSample:
	"""One `PlayerStatsEvent`, reduced to the fields the metrics read."""

	second: float
	minerals: int
	vespene: int
	workers: int
	food_used: int
	food_made: int
	resources_lost: int
	resources_killed: int
	minerals_lost_army: int
	minerals_lost_economy: int
	minerals_lost_technology: int
	army_value: int


def collect_stats(replay: Any, directory: PlayerDirectory) -> dict[int, list[StatSample]]:
	"""Groups `PlayerStatsEvent`s by player, in time order, with same-tick duplicates merged.

	The client occasionally emits two stats events on the same game second. Measured over 400
	replays: 412 of 425 duplicate-timestamp groups carried identical values, and the remaining
	13 differed only in the loss counters, i.e. a kill and its recording landed on the same tick.
	Keeping the last sample of a group is therefore both a deduplication and the more complete
	reading of that instant; keeping both would create zero-length intervals, and a zero-length
	interval with a non-zero loss becomes a phantom engagement in the battle timeline.

	Args:
	    replay: A loaded sc2reader replay (`load_level=4`).
	    directory: The replay's player directory.

	Returns:
	    A per-pid list of samples, each already converted to game seconds.
	"""
	grouped: dict[int, list[StatSample]] = collections.defaultdict(list)
	for event in replay.tracker_events:
		if type(event).__name__ != "PlayerStatsEvent":
			continue
		pid = _pid_of(event, directory)
		if pid is None:
			continue
		grouped[pid].append(
			StatSample(
				second=float(getattr(event, "second", 0.0)),
				minerals=int(getattr(event, "minerals_current", 0) or 0),
				vespene=int(getattr(event, "vespene_current", 0) or 0),
				workers=int(getattr(event, "workers_active_count", 0) or 0),
				food_used=int(getattr(event, "food_used", 0) or 0),
				food_made=int(getattr(event, "food_made", 0) or 0),
				resources_lost=int(getattr(event, "resources_lost", 0) or 0),
				resources_killed=int(getattr(event, "resources_killed", 0) or 0),
				minerals_lost_army=int(getattr(event, "minerals_lost_army", 0) or 0),
				minerals_lost_economy=int(getattr(event, "minerals_lost_economy", 0) or 0),
				minerals_lost_technology=int(
					getattr(event, "minerals_lost_technology", 0) or 0
				),
				army_value=int(getattr(event, "minerals_used_current_army", 0) or 0)
				+ int(getattr(event, "vespene_used_current_army", 0) or 0),
			)
		)
	result: dict[int, list[StatSample]] = {}
	for pid, samples in grouped.items():
		samples.sort(key=lambda sample: sample.second)
		# `dict` preserves insertion order and a later assignment for an existing key keeps the
		# original position, so this both deduplicates and stays time-ordered.
		by_second: dict[float, StatSample] = {sample.second: sample for sample in samples}
		result[pid] = list(by_second.values())
	return result


def worker_graph(samples: list[StatSample]) -> dict[str, list[float]]:
	"""Builds the worker-count curve, including the drops that mean workers were killed.

	Only drops are annotated. A rising count is ordinary production, and marking it would bury
	the signal the feature exists for: a runby that killed 20 drones shows up as one cliff.

	Args:
	    samples: One player's stat samples, time-ordered.

	Returns:
	    A series dict with `seconds` and `workers`, plus `drops` naming the worker losses.
	"""
	seconds: list[float] = []
	workers: list[int] = []
	drops: list[dict[str, Any]] = []
	previous: StatSample | None = None
	for sample in samples:
		seconds.append(round(sample.second, 1))
		workers.append(sample.workers)
		if previous is not None and sample.workers < previous.workers:
			drops.append(
				{
					"second": round(sample.second, 1),
					"from": previous.workers,
					"to": sample.workers,
					"lost": previous.workers - sample.workers,
					# The loss happened somewhere in the sampling interval, not at the sample.
					"sometimeAfter": round(previous.second, 1),
				}
			)
		previous = sample
	return {"seconds": seconds, "workers": workers, "drops": drops}


def supply_blocks(samples: list[StatSample]) -> list[dict[str, Any]]:
	"""Finds stretches where supply was capped, as bounded intervals.

	At cap means `food_used == food_made`: every point of supply is spent and none is under
	construction. Because the samples are 10 game seconds apart, the true start and end lie
	within the neighbouring gaps, so each block reports both a conservative and a generous
	duration rather than a single number that would imply precision the data does not have.

	The first sample is skipped: `food_used == food_made == 0` before anything exists would
	otherwise report a supply block from the very start of every game.

	Args:
	    samples: One player's stat samples, time-ordered.

	Returns:
	    One entry per block, with `minSeconds` and `maxSeconds` bounds.
	"""
	blocks: list[dict[str, Any]] = []
	active: StatSample | None = None
	previous: StatSample | None = None
	for sample in samples:
		at_cap = sample.food_used == sample.food_made and sample.food_made > 0
		if at_cap and active is None:
			active = sample
		elif not at_cap and active is not None:
			blocks.append(_block_bounds(active, previous, sample))
			active = None
		previous = sample
	if active is not None and previous is not None:
		blocks.append(_block_bounds(active, previous, previous, open_ended=True))
	return blocks


def _block_bounds(
	first: StatSample,
	last_at_cap: StatSample | None,
	after: StatSample,
	open_ended: bool = False,
) -> dict[str, Any]:
	"""Builds the interval for one supply block, with conservative and generous bounds.

	Args:
	    first: The first sample at which the player was capped.
	    last_at_cap: The last sample at which the player was capped.
	    after: The first sample after supply was available again.
	    open_ended: True when the game ended while still capped.

	Returns:
	    The block, with `minSeconds`/`maxSeconds` bounding the true duration.
	"""
	end = last_at_cap.second if last_at_cap is not None else first.second
	minimum = max(0.0, end - first.second)
	maximum = max(minimum, after.second - first.second)
	return {
		"sometimeAfter": round(first.second, 1),
		"clearedBy": None if open_ended else round(after.second, 1),
		"minSeconds": round(minimum, 1),
		"maxSeconds": round(maximum, 1),
		"openEnded": open_ended,
	}


def resource_differential(samples: list[StatSample]) -> dict[str, list[float]]:
	"""Builds the cumulative resources-lost differential.

	The feature list asks for a graph where zero is even, positive is in your favour and
	negative is the opponent's. That is `killed - lost` accumulated, so a rising line means
	winning the exchanges even where both sides are losing units.

	Args:
	    samples: One player's stat samples, time-ordered.

	Returns:
	    A series dict with `seconds` and the cumulative `differential`.
	"""
	seconds: list[float] = []
	differential: list[int] = []
	for sample in samples:
		# `resources_lost` and `resources_killed` are already cumulative totals written by the
		# client, so the differential is their difference, not an accumulation over samples.
		seconds.append(round(sample.second, 1))
		differential.append(sample.resources_killed - sample.resources_lost)
	return {"seconds": seconds, "differential": differential}


def engagements(samples: list[StatSample]) -> list[dict[str, Any]]:
	"""Finds moments where units were lost, as trades with a net result.

	Each entry is the change in the loss counters between two consecutive samples. A sample
	interval in which neither side lost anything is not an engagement and is skipped, which is
	what keeps the battle timeline free of the flat stretches the feature list complains about.

	The net is signed from the sampled player's point of view.

	Args:
	    samples: One player's stat samples, time-ordered.

	Returns:
	    One entry per interval in which something died.
	"""
	trades: list[dict[str, Any]] = []
	previous: StatSample | None = None
	for sample in samples:
		if previous is not None:
			lost = sample.resources_lost - previous.resources_lost
			killed = sample.resources_killed - previous.resources_killed
			if lost or killed:
				trades.append(
					{
						"second": round(sample.second, 1),
						"after": round(previous.second, 1),
						"lost": lost,
						"killed": killed,
						"net": killed - lost,
					}
				)
		previous = sample
	return trades


def build_order_from_events(replay: Any, directory: PlayerDirectory) -> dict[int, list[dict]]:
	"""Extracts each player's build order from tracker events alone, without spawningtool.

	This exists alongside spawningtool rather than replacing it. spawningtool classifies steps
	as workers and applies supply ordering, which is what a build-order guide wants, but it
	does not tell you *where* a building went. The tracker stream has the position, so this
	path is used for anything that needs placement (proxy racks, sim city, expansion timing).

	Only player-owned, non-neutral unit births count. `UnitInitEvent` is included because most
	structures are created through it rather than through `UnitBornEvent`.

	Args:
	    replay: A loaded sc2reader replay (`load_level=4`).
	    directory: The replay's player directory.

	Returns:
	    A per-pid list of steps with `second`, `name` and `position`.
	"""
	steps: dict[int, list[dict]] = collections.defaultdict(list)
	for event in replay.tracker_events:
		kind = type(event).__name__
		if kind not in ("UnitBornEvent", "UnitInitEvent"):
			continue
		name = str(getattr(event, "unit_type_name", "") or "")
		unit = getattr(event, "unit", None)
		if not name or is_neutral_unit(unit, name):
			continue
		pid = _pid_of(event, directory)
		if pid is None:
			continue
		location = getattr(event, "location", None)
		steps[pid].append(
			{
				"second": round(float(getattr(event, "second", 0.0)), 1),
				"name": name,
				"isWorker": is_worker_unit(unit, name),
				# `is_building` is what separates a structure from a unit that is *morphed*
				# through `UnitInitEvent` (Zealot, HighTemplar, Archon, CreepTumor). Without
				# it a build order reports zealots as buildings.
				"isStructure": bool(getattr(unit, "is_building", False)),
				"x": round(location[0], 1) if location else None,
				"y": round(location[1], 1) if location else None,
			}
		)
	for player_steps in steps.values():
		player_steps.sort(key=lambda step: step["second"])
	return dict(steps)


def upgrades(replay: Any, directory: PlayerDirectory) -> dict[int, list[dict]]:
	"""Collects completed upgrades per player, with cosmetic entries removed.

	Args:
	    replay: A loaded sc2reader replay (`load_level=4`).
	    directory: The replay's player directory.

	Returns:
	    A per-pid list of `{second, name}` entries.
	"""
	found: dict[int, list[dict]] = collections.defaultdict(list)
	for event in replay.tracker_events:
		if type(event).__name__ != "UpgradeCompleteEvent":
			continue
		name = str(getattr(event, "upgrade_type_name", "") or "")
		if not name or is_cosmetic_upgrade(name):
			continue
		pid = _pid_of(event, directory)
		if pid is None:
			continue
		found[pid].append({"second": round(float(getattr(event, "second", 0.0)), 1), "name": name})
	for entries in found.values():
		entries.sort(key=lambda entry: entry["second"])
	return dict(found)


def death_locations(replay: Any, directory: PlayerDirectory) -> list[dict[str, Any]]:
	"""Collects every unit death with a position, labelled by type.

	`UnitDiedEvent.unit_type_name` is None in every replay tested, so the type is recovered by
	joining the death to the birth or init that shares its `unit_id`. Without that join the
	feature is impossible: the coordinates are present, but nothing says what died.

	Worker deaths are flagged explicitly because "vulnerability to runbys" is defined by them.

	Args:
	    replay: A loaded sc2reader replay (`load_level=4`).
	    directory: The replay's player directory.

	Returns:
	    One entry per death, with `x`, `y`, `unit`, `owner` and `isWorker`.
	"""
	types: dict[int, str] = {}
	for event in replay.tracker_events:
		if type(event).__name__ not in ("UnitBornEvent", "UnitInitEvent"):
			continue
		unit_id = getattr(event, "unit_id", None)
		unit_type = str(getattr(event, "unit_type_name", "") or "")
		if unit_id is not None and unit_type:
			types[unit_id] = unit_type

	deaths: list[dict[str, Any]] = []
	for event in replay.tracker_events:
		if type(event).__name__ != "UnitDiedEvent":
			continue
		unit = getattr(event, "unit", None)
		unit_type = types.get(getattr(event, "unit_id", None), "")
		if not unit_type or is_neutral_unit(unit, unit_type):
			continue
		location = getattr(event, "location", None)
		# `UnitDiedEvent` has no ownership field of its own; `unit.owner` is the only way in.
		owner = directory.owner_of(event)
		deaths.append(
			{
				"second": round(float(getattr(event, "second", 0.0)), 1),
				"unit": unit_type,
				"x": round(location[0], 1) if location else None,
				"y": round(location[1], 1) if location else None,
				"ownerPid": owner.pid if owner else None,
				"ownerRace": owner.race if owner else None,
				"isWorker": is_worker_unit(unit, unit_type),
			}
		)
	deaths.sort(key=lambda death: death["second"])
	return deaths


def ability_casts(replay: Any, directory: PlayerDirectory) -> list[dict[str, Any]]:
	"""Collects the inject, MULE and chrono casts that the energy features measure.

	These come from `game_events`, not tracker events, and they are only present at
	`load_level=4`. The ability names are not what one would guess: a Zerg inject is recorded
	as `SpawnLarva` and a Terran MULE as `CalldownMULE`.

	Args:
	    replay: A loaded sc2reader replay (`load_level=4`).
	    directory: The replay's player directory.

	Returns:
	    One entry per cast, with `second`, `kind`, `playerPid` and `position`.
	"""
	wanted = {
		INJECT_ABILITY: "inject",
		MULE_ABILITY: "mule",
		CHRONO_ABILITY: "chrono",
	}
	casts: list[dict[str, Any]] = []
	for event in replay.game_events:
		ability = getattr(event, "ability", None)
		name = getattr(ability, "name", None) if ability is not None else None
		kind = wanted.get(name) if name else None
		if kind is None:
			continue
		player = directory.owner_of(event)
		location = getattr(event, "location", None)
		casts.append(
			{
				"second": round(float(getattr(event, "second", 0.0)), 1),
				"kind": kind,
				"playerPid": player.pid if player else None,
				"x": round(location[0], 1) if location else None,
				"y": round(location[1], 1) if location else None,
			}
		)
	casts.sort(key=lambda cast: cast["second"])
	return casts


def town_hall_positions(replay: Any, directory: PlayerDirectory) -> dict[int, list[tuple[float, float]]]:
	"""Collects each player's town-hall positions, so energy casts can be grouped by base.

	Attributing a cast to a base matters more than it sounds: two injects 5 seconds apart at two
	different hatcheries are normal play, while two injects 5 seconds apart at *one* hatchery are
	impossible. Measuring gaps across all bases at once reads normal play as heavy misuse.

	Args:
	    replay: A loaded sc2reader replay (`load_level=4`).
	    directory: The replay's player directory.

	Returns:
	    Per-pid lists of `(x, y)` town-hall positions, deduplicated.
	"""
	found: dict[int, set[tuple[float, float]]] = collections.defaultdict(set)
	for event in replay.tracker_events:
		if type(event).__name__ not in ("UnitBornEvent", "UnitInitEvent"):
			continue
		if str(getattr(event, "unit_type_name", "") or "") not in TOWN_HALL_NAMES:
			continue
		location = getattr(event, "location", None)
		pid = _pid_of(event, directory)
		if location and pid is not None:
			found[pid].add((round(float(location[0])), round(float(location[1]))))
	return {pid: sorted(positions) for pid, positions in found.items()}


def _nearest_base(
	cast: dict[str, Any], bases: list[tuple[float, float]]
) -> tuple[float, float] | None:
	"""Finds the base a cast was aimed at, by nearest town hall.

	An energy cast carries a target location, and a hatchery or nexus is the only thing it can
	meaningfully target. Nearest-neighbour is enough because town halls are tens of game units
	apart while a cast lands within a few units of its target.

	Args:
	    cast: One entry from `ability_casts`.
	    bases: That player's town-hall positions.

	Returns:
	    The matched base position, or None when there is nothing to match against.
	"""
	x, y = cast.get("x"), cast.get("y")
	if x is None or y is None or not bases:
		return None
	return min(bases, key=lambda base: (base[0] - x) ** 2 + (base[1] - y) ** 2)


def energy_timeline(
	casts: list[dict[str, Any]],
	bases: list[tuple[float, float]],
	kind: str,
	ideal_seconds: float,
) -> dict[str, Any]:
	"""Reports energy ability downtime, measured per base against the ideal cycle time.

	The feature list asks for "missed Queen/Mule/Chrono energy time": time when the ability was
	available and unused. That is the gap between two casts at one base, minus the interval a
	player with perfect uptime would have achieved. A base left uninjected for 55 seconds on a
	31.7 second cycle reports 23.3 seconds missed.

	Casts that cannot be matched to a base are pooled into one group labelled `base: null`
	rather than dropped, because silently discarding them would make downtime look better than
	it was.

	This deliberately does **not** claim to know how much energy was banked or how many queens
	were alive. The replay records neither, and inventing them would turn a checkable number
	into a plausible one.

	Args:
	    casts: Casts of one kind, as from `ability_casts`.
	    bases: That player's town-hall positions, from `town_hall_positions`.
	    kind: The kind being measured, used to label the result.
	    ideal_seconds: The energy-limited interval the casts are measured against.

	Returns:
	    A dict with the cast `seconds`, per-base `gaps`, and the total `missedSeconds`.
	"""
	grouped: dict[tuple[float, float] | None, list[float]] = collections.defaultdict(list)
	for cast in casts:
		grouped[_nearest_base(cast, bases)].append(cast["second"])

	gaps: list[dict[str, Any]] = []
	for base, seconds in sorted(grouped.items(), key=lambda item: str(item[0])):
		seconds.sort()
		for before, after in zip(seconds, seconds[1:]):
			idle = after - before - ideal_seconds
			# A negative idle means the two casts were closer together than the energy maths
			# allows, which happens when a cast is mis-attributed to the wrong base. Skipping
			# rather than clamping keeps that visible as a smaller sample count instead of a
			# suspiciously perfect uptime.
			if idle > 0:
				gaps.append(
					{
						"base": list(base) if base else None,
						"from": round(before, 1),
						"to": round(after, 1),
						"idleSeconds": round(idle, 1),
					}
				)
	return {
		"kind": kind,
		"idealSeconds": round(ideal_seconds, 1),
		"bases": len([base for base in grouped if base is not None]),
		"seconds": sorted(cast["second"] for cast in casts),
		"gaps": sorted(gaps, key=lambda gap: gap["from"]),
		"missedSeconds": round(sum(gap["idleSeconds"] for gap in gaps), 1),
	}


def army_composition(births: dict[int, list[dict]]) -> dict[int, dict[str, int]]:
	"""Counts what each player built, as a composition summary.

	Args:
	    births: Per-pid unit steps, as from `build_order_from_events`.

	Returns:
	    A per-pid mapping of unit name to count, largest first.
	"""
	composition: dict[int, dict[str, int]] = {}
	for pid, steps in births.items():
		counts = collections.Counter(step["name"] for step in steps)
		composition[pid] = dict(counts.most_common())
	return composition

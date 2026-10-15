"""Who a replay's events belong to.

Attribution is the single most error-prone part of this codebase, because sc2reader exposes
ownership through **four different fields with three different conventions**, and three of the
four are wrong in ways that still produce plausible output.

The fields, as verified against the reference library:

1. `event.control_pid` — on `UnitBornEvent` and `UnitInitEvent`. A real player id, and **0 for
   neutral units**. This is the only usable field on births.
2. `event.player` — a `Participant` on game events and on `PlayerStatsEvent`. Reliable there,
   and **absent (None) on `UnitBornEvent`, `UnitInitEvent` and `UnitDiedEvent`**. Reading
   births through `event.player` yields nothing at all rather than raising.
3. `event.pid` — a **0-based index** into `replay.players` on game events, but a **real player
   id** on `PlayerStatsEvent`. Two meanings for one name, which is why it is not used here
   except as a last-resort index fallback. A truthy/set-based test on it also drops index 0,
   i.e. the first player.
4. `event.unit.owner` — a `Participant` on the `Unit` attached to a `UnitDiedEvent`, and None
   for neutral. This is the only way to attribute a death: `UnitDiedEvent` carries no
   `control_pid` and no `player`, only `unit_id`. Reading a death's owner from
   `unit_owner_pid` finds nothing, because no such attribute exists.

`team_id` compounds this: the first team holds `team_id == 0`, so a truthiness check on it
silently discards the winning side. Teams are compared with `==`, never tested for truth.
"""

from __future__ import annotations

from typing import Any

from .replay import PlayerInfo

# Neutral/owned sentinel used by `control_pid`. Zero is the map's owner, not a player.
NEUTRAL_CONTROL_PID = 0

# Ownership fields on sc2reader events, in the order they should be consulted. Each entry is
# (attribute, is_real_player_id) so a real id is looked up in the map and the `pid` index
# fallback is consulted last.
_OWNERSHIP_FIELDS: tuple[tuple[str, bool], ...] = (
	("control_pid", True),
	("unit_controller", False),
	("player", False),
	("pid", False),
)


class PlayerDirectory:
	"""Resolves replay events to the player they belong to.

	Built once per replay and reused, because every metric needs attribution and rebuilding the
	mapping per event would be quadratic.
	"""

	def __init__(self, players: list[PlayerInfo]) -> None:
		"""Indexes the replay's players by pid and by the 0-based event index.

		Args:
		    players: The players as reported by the replay header.
		"""
		self.players = players
		self._by_pid = {player.pid: player for player in players}
		# `event.pid` indexes the list on game events, so the list order *is* that mapping.
		self._by_index = list(players)

	def by_pid(self, pid: int) -> PlayerInfo | None:
		"""Looks a player up by their real player id.

		Args:
		    pid: A `Player.pid` value, as recorded in the replay header.

		Returns:
		    The player, or None when the id is unknown.
		"""
		return self._by_pid.get(pid)

	def owner_of(self, event: Any) -> PlayerInfo | None:
		"""Returns the player an event belongs to, across both event streams.

		Consults the ownership fields in order and returns the first that resolves. A
		`control_pid` of 0 is authoritative for *neutral* and stops the search, because falling
		through would attribute map scenery to whichever player happened to be index 0.

		Args:
		    event: Any sc2reader event, from `tracker_events` or `game_events`.

		Returns:
		    The owning player, or None for neutral, unowned or unattributable events.
		"""
		for attribute, is_real_pid in _OWNERSHIP_FIELDS:
			value = getattr(event, attribute, None)
			if value is None:
				continue
			if attribute == "control_pid":
				if value == NEUTRAL_CONTROL_PID:
					return None
				if is_real_pid and isinstance(value, int):
					return self._by_pid.get(value)
			owner = getattr(value, "pid", None) if not isinstance(value, int) else value
			if owner is None:
				continue
			known = self._by_pid.get(owner)
			if known is not None:
				return known

		# A `UnitDiedEvent` carries no ownership of its own; the unit does.
		unit = getattr(event, "unit", None)
		unit_owner = getattr(unit, "owner", None) if unit is not None else None
		unit_pid = getattr(unit_owner, "pid", None) if unit_owner is not None else None
		if unit_pid is not None:
			return self._by_pid.get(unit_pid)

		# Last resort for a game event with no `player`: `pid` indexes the header list. This
		# is deliberately last, because on `PlayerStatsEvent` the same field is a real id.
		index = getattr(event, "pid", None)
		if isinstance(index, int) and 0 <= index < len(self._by_index):
			return self._by_index[index]
		return None

	def opponents(self, player: PlayerInfo) -> list[PlayerInfo]:
		"""Returns every player not on the given player's team.

		Compares `team_id` with `==` rather than testing it for truth, because the first team
		has `team_id == 0` and a truthiness test would return the given player's own team.

		Args:
		    player: The player to find opponents for.

		Returns:
		    The players on other teams.
		"""
		return [other for other in self.players if other.team != player.team]

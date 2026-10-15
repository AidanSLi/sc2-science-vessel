"""Loading replays, and the four constants that every metric depends on.

This module is the only place that talks to `sc2reader`, so the rest of the sidecar works
against plain Python values. Two things here are load-bearing and were measured, not assumed:

1. `load_level` must be 4 to get any events. At level 2 both event streams come back empty,
   and at level 0 the players, map and speed are missing too. Level 0 therefore fails
   silently rather than loudly, which is why it is not used anywhere.
2. `frames / 16` is the game clock, and the game clock is 1:1 with real time. sc2reader's own
   `replay.length` divides by an extra 1.4 for every LotV replay, so it under-reports every
   modern game. `game_seconds()` below is the only sanctioned conversion.
"""

from __future__ import annotations

import json
import pathlib
from dataclasses import dataclass, field
from typing import Any

import sc2reader
from mpyq import MPQArchive

# Frames per game second. Fixed by the engine, not read per-replay.
GAME_FPS = 16.0

# `sc2reader` events are only populated at this level. See the module docstring.
EVENT_LOAD_LEVEL = 4

# Header-only facts (players, map, speed, result) are present at this level and it is
# noticeably cheaper than 4, so library scans that do not need events use it.
HEADER_LOAD_LEVEL = 2

# The build from which replays are Legacy of the Void and the 1:1 clock applies. Taken from
# sc2reader's own `resources.py` branch (`if 34784 <= self.build`).
LOTV_BUILD = 34784

GAMEMETADATA_MEMBER = b"replay.gamemetadata.json"

# MMR outside this range is a parse artefact, not a rating: the reference library contains a
# replay on an arcade map reporting -36400.
MMR_MIN, MMR_MAX = 0, 8000

_COSMETIC_UPGRADE_MARKERS = ("spray", "reward", "dance")


def game_seconds(frames: int) -> float:
	"""Converts engine frames to seconds on the in-game clock.

	The clock has run 1:1 with real time since Legacy of the Void, so this value is also the
	seconds of unpaused play. It is deliberately a bare division with no speed argument: a
	signature that accepts a multiplier invites the pre-LotV 1.4 factor back in.

	Args:
	    frames: A raw frame count from a replay or event.

	Returns:
	    Seconds on the game clock.
	"""
	return frames / GAME_FPS


def is_cosmetic_upgrade(name: str) -> bool:
	"""Reports whether an `UpgradeCompleteEvent` name is a spray, reward or dance emote.

	These are cosmetic unlocks rather than upgrades, and they dominate the event list: one
	probe found 4 `SprayZerg`, 2 `SprayTerran` and eight different `RewardDance*` entries in a
	single game whose only real upgrade was `zerglingmovementspeed`.

	Args:
	    name: The upgrade name as reported by sc2reader.

	Returns:
	    True when the entry should be ignored by every upgrade metric.
	"""
	lowered = name.lower()
	return any(marker in lowered for marker in _COSMETIC_UPGRADE_MARKERS)


@dataclass
class PlayerInfo:
	"""One player in a replay, reduced to the fields the metrics use."""

	pid: int
	name: str
	race: str
	team: int
	result: str
	is_human: bool


@dataclass
class ReplayHeader:
	"""Everything about a replay that can be read without decoding its event streams."""

	path: str
	build: int
	base_build: int
	release: str
	game_type: str
	map_name: str
	speed: str
	frames: int
	is_ladder: bool
	date: str
	players: list[PlayerInfo] = field(default_factory=list)
	mmr: list[int] = field(default_factory=list)
	winners: list[str] = field(default_factory=list)
	error: str | None = None

	@property
	def length_seconds(self) -> float:
		"""Length of the game on the in-game clock, in seconds."""
		return game_seconds(self.frames)

	@property
	def matchup(self) -> str:
		"""The matchup, as a code like `ZvT`, or an empty string if it cannot be read.

		Built from `play_race` rather than the player's selected race so that a random player
		contributes the race they actually rolled.
		"""
		races = [player.race[:1] for player in self.players[:2]]
		if len(races) < 2 or not all(races):
			return ""
		return f"{races[0]}v{races[1]}"


def read_gamemetadata(path: pathlib.Path) -> dict[str, Any]:
	"""Reads the client-written `replay.gamemetadata.json` out of a replay archive.

	The file is a member *inside* the MPQ, not a sibling on disk, so it must go through
	`MPQArchive`; a `Path.with_suffix` lookup finds nothing and fails silently.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    The parsed metadata, or an empty dict when it is absent or malformed. Ranked games
	    are the only ones carrying an MMR, so absence is ordinary.
	"""
	try:
		raw = MPQArchive(str(path), listfile=False).read_file(GAMEMETADATA_MEMBER)
	except Exception:
		return {}
	if not raw:
		return {}
	try:
		parsed = json.loads(raw.decode("utf-8", "replace"))
	except (ValueError, UnicodeDecodeError):
		return {}
	return parsed if isinstance(parsed, dict) else {}


def extract_mmr(metadata: dict[str, Any]) -> list[int]:
	"""Pulls the believable MMR values out of parsed gamemetadata.

	`Players` is a **list** of per-player objects, not a mapping keyed by player id. Reading
	it as a dict yields no MMR at all rather than raising, which is how this went unnoticed.

	Args:
	    metadata: Parsed `replay.gamemetadata.json`.

	Returns:
	    Every MMR that is present and within a plausible range.
	"""
	players = metadata.get("Players")
	entries: list[Any] = []
	if isinstance(players, list):
		entries = players
	elif isinstance(players, dict):
		entries = list(players.values())
	found: list[int] = []
	for entry in entries:
		if not isinstance(entry, dict):
			continue
		value = entry.get("MMR")
		if isinstance(value, (int, float)) and MMR_MIN <= value <= MMR_MAX:
			found.append(int(value))
	return found


def load_header(path: str | pathlib.Path) -> ReplayHeader:
	"""Reads a replay's header-level facts without decoding events.

	Never raises. A library scan needs to report the files it cannot read rather than abort on
	the first one, because "these N replays are unreadable" is itself the useful answer.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    The header, with `error` set when the file could not be parsed.
	"""
	replay_path = pathlib.Path(path)
	header = ReplayHeader(
		path=str(replay_path),
		build=0,
		base_build=0,
		release="",
		game_type="",
		map_name="",
		speed="",
		frames=0,
		is_ladder=False,
		date="",
	)
	try:
		replay = sc2reader.load_replay(str(replay_path), load_level=HEADER_LOAD_LEVEL)
	except Exception as error:  # noqa: BLE001 - the rendered error is the value
		header.error = f"{type(error).__name__}: {error}"
		return header

	header.build = int(getattr(replay, "build", 0) or 0)
	header.base_build = int(getattr(replay, "base_build", 0) or 0)
	header.release = str(getattr(replay, "release_string", "") or "")
	header.game_type = str(getattr(replay, "game_type", "") or "")
	header.map_name = str(getattr(replay, "map_name", "") or "")
	header.speed = str(getattr(replay, "speed", "") or "")
	header.frames = int(getattr(replay, "frames", 0) or 0)
	header.is_ladder = bool(getattr(replay, "is_ladder", False))
	header.date = str(getattr(replay, "date", "") or "")
	header.players = [
		PlayerInfo(
			pid=int(getattr(player, "pid", 0) or 0),
			name=str(getattr(player, "name", "") or ""),
			race=str(getattr(player, "play_race", "") or ""),
			team=int(getattr(player, "team_id", 0) or 0),
			result=str(getattr(player, "result", "") or ""),
			is_human=bool(getattr(player, "is_human", True)),
		)
		for player in getattr(replay, "players", [])
	]
	header.winners = [
		player.name for player in header.players if player.result.lower() == "win"
	]
	header.mmr = extract_mmr(read_gamemetadata(replay_path))
	return header


def load_events(path: str | pathlib.Path):
	"""Loads a replay with its event streams decoded.

	Use this only where events are genuinely needed: it is 100-300x slower than `load_header`
	because it decodes tens of thousands of events.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    The sc2reader `Replay`, whose `tracker_events` and `game_events` are populated.
	"""
	return sc2reader.load_replay(str(path), load_level=EVENT_LOAD_LEVEL)

"""Scan a replay directory and report header-level facts about every replay.

This is a development tool, not part of the app. It exists to answer questions the app's
design depends on: which game builds are present in a library, what the matchup mix looks
like, how often MMR is recorded, and how long parsing takes. The feasibility study ran the
same shape of scan over 722 replays at roughly 3 ms each, and this reproduces it.

It parses at **details level** (`load_level=2`), which reads the MPQ header, the replay's own
metadata, and the player/map/speed blocks, but not the event streams. That keeps it fast
enough to run over a whole library, and it is why it can report length, speed and MMR without
touching the tens of thousands of events inside each file.

Usage:
    python tools/scan_library.py <replay-dir> [--limit N] [--json]

Note on time: `frames / 16` is the game clock. It has run 1:1 with real time since Legacy
of the Void, so it is also the wall-clock duration of unpaused play. `sc2reader`'s
`replay.length` is deliberately **not** used here — it divides LotV frames by an extra 1.4
(see AGENTS.md), so it under-reports every modern game.

This tool is deliberately standalone: it imports `sc2reader` and `mpyq` directly rather than
`sc2sv`, so it can be run to diagnose a parsing problem that would stop `sc2sv` itself from
being importable. The duplication of `GAME_FPS` and the MMR bounds is the price of that.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import time
from collections import Counter
from dataclasses import asdict, dataclass

import sc2reader
import sc2reader.exceptions
from mpyq import MPQArchive

# `replay.gamemetadata.json` lives *inside* the replay archive. Ranked games are the only
# ones that carry an MMR, so its absence is normal rather than an error.
GAMEMETADATA_MEMBER = b"replay.gamemetadata.json"

# Parse level 2 ("details") rather than 0. Counter-intuitively, level 0 is *not* enough to
# populate this what this tool reports: it reads the header only, so `players`, `map_name`,
# `speed` and `game_type` all come back empty and every matchup reads as blank. Level 2 is
# the cheapest level that fills them in, and the cost is the same in practice (about 2 ms per
# replay on the reference library) because the expensive part is decompressing the MPQ, not
# the details block. Keep this at 2 unless a much larger library makes the trimming matter.
SC2READER_LOAD_LEVEL = 2

# Frames per second of game time. A fixed engine constant, not a per-replay value, though it
# is read from the replay where available so a future engine change is visible rather than
# silently assumed.
DEFAULT_GAME_FPS = 16.0

# MMR outside this range is a parsing artifact rather than a rating. One replay in the
# reference library reports -36400 (an arcade map), which would wreck any average.
MMR_MIN, MMR_MAX = 0, 8000


@dataclass
class ReplaySummary:
	"""Header-level facts about one replay, as reported by `scan_library`."""

	path: str
	map_name: str
	game_type: str
	build: int
	release: str
	speed: str
	length_in_game_seconds: float
	date: str
	is_ladder: bool
	matchup: str
	players: list[dict]
	mmr: list[int]
	error: str | None = None


def read_gamemetadata(path: pathlib.Path) -> dict:
	"""Reads `replay.gamemetadata.json` out of the replay's MPQ archive.

	Returns an empty dict when the member is absent or unreadable. The file is a member of
	the archive, *not* a sibling file on disk, so it must be read through `MPQArchive`; the
	obvious `path.with_suffix(...)` lookup silently finds nothing.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    The parsed metadata, or `{}` when it is missing or malformed.
	"""
	try:
		archive = MPQArchive(str(path), listfile=False)
		raw = archive.read_file(GAMEMETADATA_MEMBER)
	except Exception:
		return {}
	if not raw:
		return {}
	try:
		return json.loads(raw.decode("utf-8", "replace"))
	except (ValueError, UnicodeDecodeError):
		return {}


def extract_mmr(metadata: dict) -> list[int]:
	"""Pulls the plausible MMR values out of a replay's gamemetadata.

	`Players` is a **list** of per-player objects, not a mapping keyed by player id — the
	documented shape of this file is misleading and reading it as a dict silently yields no
	MMR at all rather than raising. Each entry looks like
	`{"PlayerID": 1, "MMR": 3558, "Result": "Loss", "AssignedRace": "Zerg"}`, and `MMR`
	is only present in ranked games.

	Args:
	    metadata: The parsed `replay.gamemetadata.json` contents.

	Returns:
	    Every MMR value on the players that has one and is in a believable range.
	"""
	if not isinstance(metadata, dict):
		return []
	players = metadata.get("Players")
	# Accept both shapes defensively; only the list form has been observed in reality.
	entries: list[object] = []
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


def summarise(path: pathlib.Path) -> ReplaySummary:
	"""Parses one replay at header level and returns the summary.

	Never raises: a library scan should report the unreadable files rather than abort on the
	first corrupt replay, because "these N files cannot be read" is itself the useful answer.

	Args:
	    path: Path to the `.SC2Replay` file.

	Returns:
	    The summary, with `error` set when the file could not be parsed.
	"""
	blank = ReplaySummary(
		path=str(path),
		map_name="",
		game_type="",
		build=0,
		release="",
		speed="",
		length_in_game_seconds=0.0,
		date="",
		is_ladder=False,
		matchup="",
		players=[],
		mmr=[],
	)
	try:
		replay = sc2reader.load_replay(str(path), load_level=SC2READER_LOAD_LEVEL)
	except Exception as error:  # noqa: BLE001 - the error text is the report
		blank.error = f"{type(error).__name__}: {error}"
		return blank

	fps = float(getattr(replay, "game_fps", None) or DEFAULT_GAME_FPS)
	races = [getattr(player, "play_race", "") or "" for player in replay.players]
	# A one-character-per-race key like `ZvT`, built from the same rule the app uses so the
	# scanner and the UI agree on what a matchup is called.
	matchup = f"{races[0][:1]}v{races[1][:1]}" if len(races) >= 2 and all(races[:2]) else ""
	metadata = read_gamemetadata(path)

	return ReplaySummary(
		path=str(path),
		map_name=str(getattr(replay, "map_name", "") or ""),
		game_type=str(getattr(replay, "game_type", "") or ""),
		build=int(getattr(replay, "build", 0) or 0),
		release=str(getattr(replay, "release_string", "") or ""),
		speed=str(getattr(replay, "speed", "") or ""),
		length_in_game_seconds=float(getattr(replay, "frames", 0) or 0) / fps,
		date=str(getattr(replay, "date", "") or ""),
		is_ladder=bool(getattr(replay, "is_ladder", False)),
		matchup=matchup,
		players=[
			{
				"name": getattr(player, "name", ""),
				"race": getattr(player, "play_race", ""),
				"result": getattr(player, "result", ""),
			}
			for player in replay.players
		],
		mmr=extract_mmr(metadata),
	)


def main() -> int:
	"""Scans the directory given on the command line and prints a report.

	Returns:
	    A process exit code: 0 on success, 1 when nothing could be read.
	"""
	parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	parser.add_argument("directory", type=pathlib.Path, help="directory to scan (recursively)")
	parser.add_argument("--limit", type=int, default=0, help="stop after N replays")
	parser.add_argument("--json", action="store_true", help="emit JSON instead of a table")
	args = parser.parse_args()

	if not args.directory.is_dir():
		print(f"not a directory: {args.directory}", file=sys.stderr)
		return 1

	paths = sorted(args.directory.rglob("*.SC2Replay"))
	if args.limit:
		paths = paths[: args.limit]
	if not paths:
		print(f"no .SC2Replay files under {args.directory}", file=sys.stderr)
		return 1

	started = time.monotonic()
	summaries = [summarise(path) for path in paths]
	elapsed = time.monotonic() - started

	if args.json:
		print(json.dumps([asdict(s) for s in summaries], indent=1))
		return 0

	ok = [s for s in summaries if not s.error]
	failed = [s for s in summaries if s.error]

	print(f"scanned {len(summaries)} replays in {elapsed:.2f}s ({elapsed / len(summaries) * 1000:.0f} ms each)")
	print(f"parsed {len(ok)}, failed {len(failed)}")
	print()

	builds = Counter(s.build for s in ok)
	matchups = Counter(s.matchup for s in ok if s.matchup)
	speeds = Counter(s.speed for s in ok)
	with_mmr = [s for s in ok if s.mmr]
	lengths = sorted(s.length_in_game_seconds for s in ok)

	print("game builds:")
	for build, count in builds.most_common(10):
		print(f"  {build:>7}  {count:4d}")
	print()
	print("matchups:")
	for matchup, count in matchups.most_common(12):
		print(f"  {matchup:>5}  {count:4d}")
	print()
	print("game speeds: " + ", ".join(f"{speed or '(none)'} x{count}" for speed, count in speeds.most_common()))
	print(f"ladder games: {sum(1 for s in ok if s.is_ladder)}/{len(ok)}")
	print(f"replays with MMR: {len(with_mmr)}/{len(ok)}")
	if lengths:
		median = lengths[len(lengths) // 2]
		print(f"length (game seconds): min {lengths[0]:.0f}, median {median:.0f}, max {lengths[-1]:.0f}")

	if failed:
		print()
		print("failures:")
		for summary in failed[:10]:
			print(f"  {pathlib.Path(summary.path).name}: {summary.error}")
	return 0


if __name__ == "__main__":
	raise SystemExit(main())

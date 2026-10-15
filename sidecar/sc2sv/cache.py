"""An on-disk cache of per-replay analysis payloads.

Why this exists: analysing a replay costs 0.3-1 s of event decoding plus a second, slower
`spawningtool` parse, and the app wants its matches list to show metrics for hundreds of
replays at once. Recomputing those on every visit would make the list either slow to paint or
permanently behind a progress bar for work the user has already waited through once. Caching
the finished payload makes the second visit instant and keeps the expensive path off the UI's
critical path.

Entries are keyed by the replay's path **plus its size and mtime**, not by path alone. A replay
path is stable — the game writes one file per game and never rewrites it — but a user can copy
a different replay over an old name, and a cache that answers with the previous game's numbers
for a real replay is worse than no cache at all. Including the size and mtime makes a swapped
file a cache miss.

The cache is a plain directory of JSON files rather than SQLite. Every entry is
write-once-per-replay and read whole, so a database would add a dependency and a locking story
without making anything faster. Corruption is handled by treating an unreadable entry as a miss
and overwriting it.
"""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
from typing import Any

# Environment variable that relocates the cache, so tests never write to a real user's cache
# directory and so a developer can throw it away without hunting for it.
CACHE_DIR_ENV = "SC2SV_CACHE_DIR"

# Suffix for one cached payload. JSON, because the payload already is JSON and an extra format
# would only be something else to get wrong.
CACHE_SUFFIX = ".analysis.json"

# Bumped by hand whenever the shape or the *meaning* of an analysis payload changes — a renamed
# key, a corrected constant, a metric that starts excluding something it used to include.
#
# Revision 4 merged energy timelines per player instead of pooling their casts and bases. On a
# real 1v1 ZvZ the pooled version reported 2116.5 s of idle inject time where per-player
# measurement gives 2578.7 s — casts were being credited to the wrong player's hatchery.
# Revision 3 corrected two more energy bugs: ability names are now read from `event.ability_name`
# (smart-cast injects were recorded as `RightClick`, so 84% of injects were invisible and the
# reported downtime was overstated in the player's favour), and casts are no longer pooled across
# players of a race. Revision 2 was the initial key.
#
# The key must include this: a cache keyed on the replay alone made every metric bug permanent
# for replays already analysed, and the symptom was the worst kind — an app confidently showing
# numbers that the current code no longer produces, with no way to tell a stale payload from a
# real result. Forgetting to bump it costs one confusing bug; not having it at all costs every
# future fix.
CACHE_REVISION = 4


def cache_dir() -> pathlib.Path:
	"""Returns the directory the cache lives in, creating it if necessary.

	Honours `SC2SV_CACHE_DIR` first, then the platform cache directory. Creating the directory
	here rather than at write time means a caller that only reads still gets a usable path and
	does not have to special-case a missing directory.

	Returns:
	    The cache directory.
	"""
	override = os.environ.get(CACHE_DIR_ENV)
	if override:
		root = pathlib.Path(override)
	else:
		base = os.environ.get("XDG_CACHE_HOME") or str(pathlib.Path.home() / ".cache")
		root = pathlib.Path(base) / "sc2-science-vessel"
	root.mkdir(parents=True, exist_ok=True)
	return root


def cache_key(path: str | pathlib.Path) -> str:
	"""Builds the cache key for a replay file.

	The key is a hash of the absolute path plus the file's size and modification time plus
	`CACHE_REVISION`, so the directory listing stays flat, the filename stays filesystem-safe, a
	replay replaced by a different file at the same path is a miss rather than a stale hit, and a
	change to the analyser itself invalidates every entry.

	Args:
	    path: The replay file.

	Returns:
	    A hex digest. A path that cannot be stat'ed still yields a key, so a caller asking about
	    a vanished file gets a miss instead of an exception.
	"""
	resolved = pathlib.Path(path).expanduser().resolve()
	try:
		stat = resolved.stat()
		identity = f"{resolved}\0{stat.st_size}\0{stat.st_mtime_ns}\0{CACHE_REVISION}"
	except OSError:
		identity = f"{resolved}\0missing\0{CACHE_REVISION}"
	return hashlib.sha256(identity.encode("utf-8")).hexdigest()


def cache_path(path: str | pathlib.Path) -> pathlib.Path:
	"""Returns the file one replay's cached payload would live in.

	Args:
	    path: The replay file.

	Returns:
	    The cache entry's path, whether or not it exists.
	"""
	return cache_dir() / f"{cache_key(path)}{CACHE_SUFFIX}"


def load(path: str | pathlib.Path) -> dict[str, Any] | None:
	"""Reads a replay's cached analysis payload.

	Args:
	    path: The replay file.

	Returns:
	    The cached payload, or None when there is no entry or the entry cannot be read. A
	    damaged entry is deliberately indistinguishable from a missing one: the caller's
	    response is the same (analyse the replay), so reporting the difference would only be
	    noise, and the next `store` overwrites the bad file anyway.
	"""
	entry = cache_path(path)
	try:
		with entry.open("r", encoding="utf-8") as handle:
			payload = json.load(handle)
	except (OSError, ValueError):
		return None
	if not isinstance(payload, dict):
		return None
	return payload


def store(path: str | pathlib.Path, payload: dict[str, Any]) -> None:
	"""Writes a replay's analysis payload into the cache.

	The write goes to a temporary file in the same directory and is then moved into place, so a
	process killed mid-write leaves either the old entry or no entry, never a truncated one that
	would parse as valid JSON and then fail somewhere downstream.

	Args:
	    path: The replay the payload describes.
	    payload: The analysis payload.
	"""
	entry = cache_path(path)
	temporary = entry.with_suffix(entry.suffix + ".tmp")
	try:
		with temporary.open("w", encoding="utf-8") as handle:
			json.dump(payload, handle, separators=(",", ":"))
		temporary.replace(entry)
	except OSError:
		# A cache that cannot be written is a performance problem, never a correctness one: the
		# caller already has the payload it computed. Swallowing the error keeps a read-only or
		# full disk from turning a successful analysis into a failure.
		temporary.unlink(missing_ok=True)


def clear() -> int:
	"""Deletes every cached payload.

	Returns:
	    How many entries were removed.
	"""
	removed = 0
	for entry in cache_dir().glob(f"*{CACHE_SUFFIX}"):
		try:
			entry.unlink()
			removed += 1
		except OSError:
			continue
	return removed

"""Pytest configuration and the fixture-replay resolver.

Fixture replays are deliberately not committed — they are large and user-specific — so every
metric test has to cope with their absence. The rule recorded in `AGENTS.md` is that a missing
fixture skips with a message naming the file rather than failing, so a fresh clone goes green.
This module owns that decision so no test has to reimplement it.

The replay directory is searched in a fixed order and overridable with `SC2SV_FIXTURE_REPLAYS`,
because the reference machine's library lives inside a Wine prefix under a path that is not
worth hardcoding into every test.
"""

from __future__ import annotations

import os
import pathlib

import pytest

REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
BUNDLED_FIXTURES = REPO_ROOT / "tests" / "fixtures" / "replays"

# Extra places a fixture might be found on a developer's machine: the working replay folder
# under the user's documents, and the Wine prefix SC2 writes to on Linux. Checked in order,
# and each one is optional. An environment variable overrides the whole search so a different
# machine can point at its own library without editing this file.
SEARCH_ROOTS = (
	pathlib.Path.home() / "Documents" / "SC2 Replays",
	pathlib.Path.home()
	/ "Faugus"
	/ "battlenet"
	/ "drive_c"
	/ "users"
	/ "steamuser"
	/ "Documents"
	/ "StarCraft II"
	/ "Accounts",
)


def _search_roots() -> list[pathlib.Path]:
	"""Lists the directories a fixture replay may be found under.

	Returns:
	    The override root when `SC2SV_FIXTURE_REPLAYS` is set, otherwise the bundled fixture
	    directory followed by the machine-local replay locations that exist.
	"""
	override = os.environ.get("SC2SV_FIXTURE_REPLAYS")
	if override:
		return [pathlib.Path(override).expanduser()]
	roots = [BUNDLED_FIXTURES]
	roots.extend(root for root in SEARCH_ROOTS if root.exists())
	return roots


def find_replay(*names: str) -> pathlib.Path | None:
	"""Finds the first existing replay among a list of candidate names.

	Names are matched as **substrings** of the filename rather than as exact names, because the
	reference library prefixes every file with a date and the fixtures are described by their
	distinguishing part (`patch_5_0_16`, `Gemini VS`). A name that already ends in
	`.SC2Replay` is matched exactly, so a literal filename still resolves to itself.

	Args:
	    *names: Name fragments to look for, in order of preference.

	Returns:
	    The path of the first match, or None when none is found. When several files match, the
	    smallest is returned, because the bundled fixtures are the trimmed ones and a smaller
	    file parses faster.
	"""
	for name in names:
		for root in _search_roots():
			matches: list[pathlib.Path] = []
			for candidate in root.rglob("*.SC2Replay"):
				if not candidate.is_file():
					continue
				if name in candidate.name or name == candidate.name:
					matches.append(candidate)
			if matches:
				return min(matches, key=lambda path: path.stat().st_size)
	return None


def load_fixture(*names: str):
	"""Loads a fixture replay with events decoded, skipping when it is unavailable.

	The replay directory holds hundreds of files, so a test that only needs "any ZvT from the
	library" can pass several candidate names and take whichever exists.

	Args:
	    *names: Candidate filenames, in order of preference.

	Returns:
	    The loaded sc2reader replay at `load_level=4`.
	"""
	import sc2reader

	path = find_replay(*names)
	if path is None:
		pytest.skip(
			"fixture replay not available; copy one into "
			f"{BUNDLED_FIXTURES} or set SC2SV_FIXTURE_REPLAYS "
			f"(looked for: {', '.join(names)})"
		)
	return sc2reader.load_replay(str(path), load_level=4)


def fixture_header(*names: str):
	"""Loads a fixture replay at header level, skipping when it is unavailable.

	Args:
	    *names: Candidate filenames, in order of preference.

	Returns:
	    The loaded sc2reader replay at `load_level=2`.
	"""
	import sc2reader

	path = find_replay(*names)
	if path is None:
		pytest.skip(
			"fixture replay not available; copy one into "
			f"{BUNDLED_FIXTURES} or set SC2SV_FIXTURE_REPLAYS "
			f"(looked for: {', '.join(names)})"
		)
	return sc2reader.load_replay(str(path), load_level=2)


@pytest.fixture(scope="session")
def any_1v1_replay_path() -> pathlib.Path:
	"""Provides the path of any parseable 1v1 replay available on this machine.

	Returns:
	    A path to a real replay. Skips when the machine has none.
	"""
	for root in _search_roots():
		for candidate in sorted(root.rglob("*.SC2Replay")):
			if candidate.is_file():
				return candidate
	pytest.skip("no .SC2Replay fixture found on this machine")

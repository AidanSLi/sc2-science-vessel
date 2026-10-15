"""Command line interface to the sidecar.

`sc2sv` is the sidecar's only entry point, and it exists for three reasons: it makes the
metric layer testable and usable before any UI exists (ADR 0001), it gives the Tauri shell a
single well-known command to invoke, and it keeps the app's analysis reproducible from a shell
when a number looks wrong.

Every subcommand writes JSON to stdout and nothing else, so the output is always pipeable.
Progress and diagnostics go to stderr.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import time
from typing import Any

from .analyze import analyse_replay
from .replay import load_header


def _print_json(payload: Any) -> None:
	"""Writes a payload to stdout as JSON.

	Args:
	    payload: Any JSON-serialisable value.
	"""
	json.dump(payload, sys.stdout, indent=1, sort_keys=False)
	sys.stdout.write("\n")


def cmd_scan(args: argparse.Namespace) -> int:
	"""Scans a directory of replays and reports header-level facts.

	Args:
	    args: Parsed arguments carrying `directory`, `limit` and `json`.

	Returns:
	    A process exit code.
	"""
	directory = pathlib.Path(args.directory)
	if not directory.is_dir():
		print(f"not a directory: {directory}", file=sys.stderr)
		return 1
	paths = sorted(directory.rglob("*.SC2Replay"))
	if args.limit:
		paths = paths[: args.limit]
	if not paths:
		print(f"no .SC2Replay files under {directory}", file=sys.stderr)
		return 1

	started = time.monotonic()
	headers = [load_header(path) for path in paths]
	elapsed = time.monotonic() - started

	if args.json:
		_print_json([header.__dict__ | {"lengthSeconds": header.length_seconds, "matchup": header.matchup} for header in headers])
		return 0

	ok = [header for header in headers if header.error is None]
	failed = [header for header in headers if header.error is not None]
	print(f"scanned {len(headers)} replays in {elapsed:.2f}s", file=sys.stderr)
	print(f"parsed {len(ok)}, failed {len(failed)}", file=sys.stderr)

	import collections

	print("builds: " + ", ".join(f"{b}x{n}" for b, n in collections.Counter(h.build for h in ok).most_common(8)))
	print("matchups: " + ", ".join(f"{m}x{n}" for m, n in collections.Counter(h.matchup for h in ok if h.matchup).most_common(10)))
	print(f"ladder {sum(1 for h in ok if h.is_ladder)}/{len(ok)}, with mmr {sum(1 for h in ok if h.mmr)}/{len(ok)}")
	if failed:
		for header in failed[:5]:
			print(f"  FAILED {pathlib.Path(header.path).name}: {header.error}", file=sys.stderr)
	return 0


def cmd_analyze(args: argparse.Namespace) -> int:
	"""Analyses one replay and writes the prepared payload as JSON.

	Args:
	    args: Parsed arguments carrying `replay` and optional `summary`.

	Returns:
	    A process exit code: 1 when the replay cannot be parsed.
	"""
	path = pathlib.Path(args.replay)
	if not path.is_file():
		print(f"not a file: {path}", file=sys.stderr)
		return 1
	started = time.monotonic()
	try:
		payload = analyse_replay(path)
	except ValueError as error:
		print(str(error), file=sys.stderr)
		return 1
	elapsed = time.monotonic() - started

	if args.summary:
		print(f"{pathlib.Path(payload['path']).name}  {payload['matchup']}  {payload['map']}", file=sys.stderr)
		print(f"  {payload['lengthSeconds']:.0f}s game clock, build {payload['build']}", file=sys.stderr)
		for player in payload["players"]:
			workers = player["workers"]["workers"]
			print(
				f"  {player['name']} ({player['race']}) {player['result']}: "
				f"{len(player['engagements'])} engagements, "
				f"{sum(player['composition'].values())} units, "
				f"peak {max(workers) if workers else 0} workers, "
				f"{len(player['supplyBlocks'])} supply blocks",
				file=sys.stderr,
			)
		energy = payload["energy"]
		print(
			f"  missed: inject {energy['inject']['missedSeconds']}s, "
			f"mule {energy['mule']['missedSeconds']}s, "
			f"chrono {energy['chrono']['missedSeconds']}s",
			file=sys.stderr,
		)
		print(f"  analysed in {elapsed:.2f}s", file=sys.stderr)
	else:
		_print_json(payload)
	return 0


def cmd_build_order(args: argparse.Namespace) -> int:
	"""Prints a replay's build order as extracted by spawningtool.

	Kept separate from `analyze` because spawningtool's step list is what a build-order guide
	wants (workers flagged, supply-ordered) while the tracker-derived steps are what placement
	features want. Mixing them into one payload would force every consumer to guess which.

	Args:
	    args: Parsed arguments carrying `replay` and optional `pid`.

	Returns:
	    A process exit code.
	"""
	from spawningtool.parser import parse_replay

	path = pathlib.Path(args.replay)
	if not path.is_file():
		print(f"not a file: {path}", file=sys.stderr)
		return 1
	result = parse_replay(str(path))
	if not result:
		print(f"spawningtool could not parse {path}", file=sys.stderr)
		return 1
	build = result.get("build", {})
	if args.pid is not None:
		build = {key: value for key, value in build.items() if str(key) == str(args.pid)}
	if args.json:
		_print_json({"build": build, "players": result.get("players", {})})
	else:
		_print_json(build)
	return 0


def build_parser() -> argparse.ArgumentParser:
	"""Builds the argument parser for the `sc2sv` command.

	Returns:
	    The configured parser.
	"""
	parser = argparse.ArgumentParser(
		prog="sc2sv",
		description="Replay analysis for sc2-science-vessel. JSON on stdout.",
	)
	subparsers = parser.add_subparsers(dest="command", required=True)

	scan = subparsers.add_parser("scan", help="report header facts for every replay in a directory")
	scan.add_argument("directory")
	scan.add_argument("--limit", type=int, default=0)
	scan.add_argument("--json", action="store_true")
	scan.set_defaults(func=cmd_scan)

	analyze = subparsers.add_parser("analyze", help="print the full analysis payload for one replay")
	analyze.add_argument("replay")
	analyze.add_argument("--summary", action="store_true", help="print a human summary to stderr instead of JSON")
	analyze.set_defaults(func=cmd_analyze)

	order = subparsers.add_parser("build-order", help="print spawningtool's build order for one replay")
	order.add_argument("replay")
	order.add_argument("--pid", type=int, default=None)
	order.add_argument("--json", action="store_true")
	order.set_defaults(func=cmd_build_order)
	return parser


def main(argv: list[str] | None = None) -> int:
	"""Runs the CLI.

	Args:
	    argv: Arguments to parse; defaults to `sys.argv[1:]`.

	Returns:
	    A process exit code.
	"""
	args = build_parser().parse_args(argv)
	return int(args.func(args))


if __name__ == "__main__":
	raise SystemExit(main())

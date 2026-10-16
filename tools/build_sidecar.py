#!/usr/bin/env python3
"""Freezes the Python analysis sidecar into a single executable for bundling.

Why this exists (ADR 0001): the shell never parses a replay, `sc2reader` does, and it is a
Python library. An installed app therefore has to carry a Python interpreter, and the only
supported way to do that without asking the user to install anything is to freeze one in.

The output is written where Tauri's `externalBin` looks for it, named with the target triple,
because that is the name the bundler resolves. Getting the name wrong produces a build that
succeeds and an app that cannot analyse anything, so the filename is computed here rather than
left to a shell command in CI.

`sc2reader` ships three JSON data files (unit costs and build times). PyInstaller cannot see a
data file it is not told about, and a missing one makes replay parsing fail at *runtime* with a
file-not-found deep inside the library, so `--collect-data` is not optional.

Usage:
    uv run --with pyinstaller python tools/build_sidecar.py

Writes `src-tauri/sc2sv-sidecar-<target-triple>` (`.exe` on Windows).
"""

from __future__ import annotations

import argparse
import platform
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SIDECAR_DIR = REPO_ROOT / "sidecar"
OUTPUT_DIR = REPO_ROOT / "src-tauri"

# Must match `SIDECAR_NAME` in src-tauri/src/sidecar.rs and the `externalBin` entry in
# tauri.conf.json. Tauri appends the target triple itself when resolving, and strips it again
# when placing the file in the bundle, so the base name is the same on every platform.
SIDECAR_NAME = "sc2sv-sidecar"

# The packages whose *data* files must travel with the code. sc2reader reads these at runtime
# for unit costs/build times; without them every metric that needs a cost is wrong or missing.
DATA_PACKAGES = ("sc2reader",)


def target_triple() -> str:
	"""Returns the Rust target triple this machine builds for, as Tauri names it.

	Asking `rustc` rather than guessing from `platform.machine()` matters because Tauri resolves
	`externalBin` by this exact string: a mismatch is a silent "sidecar not found" at run time
	rather than a build error.

	Returns:
	    The host target triple, e.g. `x86_64-unknown-linux-gnu` or `x86_64-pc-windows-msvc`.
	"""
	result = subprocess.run(
		["rustc", "-vV"],
		capture_output=True,
		text=True,
		check=True,
	)
	for line in result.stdout.splitlines():
		if line.startswith("host:"):
			return line.split(":", 1)[1].strip()
	raise SystemExit("could not read the host target triple from `rustc -vV`")


def frozen_name(triple: str) -> str:
	"""Returns the filename Tauri's bundler expects for this platform.

	Args:
	    triple: The Rust target triple.

	Returns:
	    `sc2sv-sidecar-<triple>`, with `.exe` appended on Windows.
	"""
	suffix = ".exe" if platform.system() == "Windows" else ""
	return f"{SIDECAR_NAME}-{triple}{suffix}"


def build(triple: str, clean: bool) -> Path:
	"""Runs PyInstaller and moves its output to the name Tauri resolves.

	Args:
	    triple: The Rust target triple, used to name the output.
	    clean: Whether to remove PyInstaller's `build/` and `dist/` scratch dirs first.

	Returns:
	    The path to the frozen sidecar.
	"""
	work = REPO_ROOT / "build" / "sidecar"
	dist = work / "dist"
	if clean:
		shutil.rmtree(work, ignore_errors=True)

	command = [
		sys.executable,
		"-m",
		"PyInstaller",
		"--noconfirm",
		"--onefile",
		"--name",
		SIDECAR_NAME,
		# Paths are captured at analysis time, so the build must run from a directory where
		# `sc2sv` is importable. The sidecar dir is the package's parent by construction.
		"--paths",
		str(SIDECAR_DIR),
		"--distpath",
		str(dist),
		"--workpath",
		str(work / "build"),
		"--specpath",
		str(work),
	]
	for package in DATA_PACKAGES:
		command += ["--collect-data", package]
	# The entry point must be a module that imports the package rather than a file inside it:
	# PyInstaller runs its entry script as `__main__`, and `cli.py` uses relative imports, so
	# pointing at it directly fails with "attempted relative import with no known parent package".
	# The shim calls the same `sc2sv.cli:main` the console script does.
	command += [str(SIDECAR_DIR / "sc2sv_sidecar_entry.py")]

	subprocess.run(command, cwd=REPO_ROOT, check=True)

	produced = dist / (SIDECAR_NAME + (".exe" if platform.system() == "Windows" else ""))
	if not produced.is_file():
		raise SystemExit(f"PyInstaller did not produce {produced}")

	destination = OUTPUT_DIR / frozen_name(triple)
	shutil.copy2(produced, destination)
	destination.chmod(0o755)
	return destination


def main() -> int:
	"""Parses arguments, builds the sidecar and reports where it landed.

	Returns:
	    A process exit code.
	"""
	parser = argparse.ArgumentParser(description=__doc__)
	parser.add_argument(
		"--no-clean",
		action="store_true",
		help="reuse PyInstaller's scratch directory instead of rebuilding from scratch",
	)
	args = parser.parse_args()

	triple = target_triple()
	print(f"freezing the sidecar for {triple}", file=sys.stderr)
	destination = build(triple, clean=not args.no_clean)
	size_mb = destination.stat().st_size / (1024 * 1024)
	print(f"wrote {destination} ({size_mb:.1f} MiB)", file=sys.stderr)
	return 0


if __name__ == "__main__":
	raise SystemExit(main())

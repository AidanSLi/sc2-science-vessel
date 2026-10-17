#!/usr/bin/env python3
"""Builds the resident watcher and stages it where Tauri's bundler will find it.

## Why the watcher goes through `externalBin`

Tauri bundles exactly one binary: the one named in `Cargo.toml`. The watcher is a second
program, and it must end up **beside** the GUI, because that is how the watcher finds the GUI
at runtime (a sibling lookup, no configuration). `externalBin` is the only mechanism Tauri
offers for shipping an extra executable, and it happens to do precisely that: it installs the
file into the same directory as the main binary.

The cost is that `externalBin` entries must carry the host target triple in their filename,
which Tauri strips on install. So this script builds the watcher crate and copies the result to
`src-tauri/sc2sv-watch-<triple>`, which is the file the bundle then picks up.

## Why a release build regardless of the app's profile

The watcher is the process that runs for the whole session, and it is compiled with `lto`,
`opt-level = "s"`, `panic = "abort"` and `strip` in its own profile. Building it as a debug
binary would put all of that compile time to waste and inflate the resident process — the one
thing its size requirement is about. The app's `--debug` flag therefore does not apply here.

## Usage

    python3 tools/build_watcher.py [--check]

`--check` builds nothing and only reports whether a staged watcher already exists, which is what
CI uses to fail early with a clear message rather than at bundle time.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WATCHER_DIR = REPO_ROOT / "src-tauri" / "watcher-bin"
STAGING_DIR = REPO_ROOT / "src-tauri"
CRATE_NAME = "sc2sv-watch"


def target_triple() -> str:
	"""Returns the host target triple, as `rustc` reports it.

	Read from the compiler rather than guessed from the platform, because the staged filename
	must match what Tauri expects exactly, and a mismatch is reported only at bundle time when
	the file is not found.

	@returns The triple, e.g. `x86_64-unknown-linux-gnu`.
	"""
	output = subprocess.run(
		["rustc", "-vV"], capture_output=True, text=True, check=True
	).stdout
	for line in output.splitlines():
		if line.startswith("host: "):
			return line.removeprefix("host: ").strip()
	raise SystemExit("could not read the host triple from `rustc -vV`")


def staging_path(triple: str) -> Path:
	"""Returns where the built watcher must be copied for the bundler to find it.

	@param triple The host target triple.
	@returns The staged path, with `.exe` on Windows as Tauri expects.
	"""
	suffix = ".exe" if sys.platform == "win32" else ""
	return STAGING_DIR / f"{CRATE_NAME}-{triple}{suffix}"


def build(triple: str) -> Path:
	"""Builds the watcher crate and copies it to its staging path.

	The crate is a direct `cargo build` rather than part of a workspace, deliberately: joining
	the two crates into one workspace would make the watcher share the GUI's feature resolution
	and build profile, and the point of the split is that the two have nothing in common.

	@param triple The host target triple.
	@returns The path of the built binary.
	"""
	print(f"building the watcher for {triple}", flush=True)
	subprocess.run(
		["cargo", "build", "--release", "--manifest-path", str(WATCHER_DIR / "Cargo.toml")],
		check=True,
	)
	built = WATCHER_DIR / "target" / "release" / (
		f"{CRATE_NAME}.exe" if sys.platform == "win32" else CRATE_NAME
	)
	if not built.is_file():
		raise SystemExit(f"cargo reported success but {built} does not exist")

	staged = staging_path(triple)
	shutil.copy2(built, staged)
	size = staged.stat().st_size
	print(f"staged {staged.relative_to(REPO_ROOT)} ({size:,} bytes)")
	return staged


def main() -> int:
	"""Builds the watcher unless asked only to check for a staged copy.

	@returns The process exit code.
	"""
	parser = argparse.ArgumentParser(description=__doc__)
	parser.add_argument(
		"--check",
		action="store_true",
		help="only report whether a staged watcher exists; do not build",
	)
	args = parser.parse_args()

	triple = target_triple()
	staged = staging_path(triple)

	if args.check:
		if staged.is_file():
			print(f"staged watcher present: {staged.relative_to(REPO_ROOT)}")
			return 0
		print(
			f"no staged watcher at {staged.relative_to(REPO_ROOT)}; "
			"run `python3 tools/build_watcher.py`",
			file=sys.stderr,
		)
		return 1

	build(triple)
	return 0


if __name__ == "__main__":
	raise SystemExit(main())

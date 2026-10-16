"""Frozen-build entry point for the sidecar.

PyInstaller executes its entry script as `__main__`, so pointing it at `sc2sv/cli.py` directly
breaks that module's own relative imports (`from . import builds, cache`) with
`ImportError: attempted relative import with no known parent package`. This file exists only to
import the CLI *as a package module* and call it, which keeps the frozen binary running exactly
the same code path as the `sc2sv` console script (`sc2sv = sc2sv.cli:main` in pyproject.toml)
instead of a second, parallel entry point.

It is deliberately not part of the `sc2sv` package: it is build scaffolding, and keeping it out
means the package that gets imported during development is unchanged.
"""

from __future__ import annotations

from sc2sv.cli import main

if __name__ == "__main__":
	raise SystemExit(main())

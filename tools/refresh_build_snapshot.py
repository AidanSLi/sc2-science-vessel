"""Scrape SpawningTool's public build-order pages into the bundled snapshot.

This is the **only** component in the project allowed to touch the network. Replay analysis
stays strictly offline; build-order *search* is the one feature that cannot, because the
corpus lives on someone else's website. See `docs/adr/0002` for the decision and the rule
that a failure here must degrade to "keep the existing snapshot" rather than break the app.

There is no JSON API. Checked on 2026-10:
`/build/?format=json` returns `text/html` with the parameter ignored, `/build/feed/` and
`/build/api/` are 404, and `/api/` redirects to a login page. So the HTML is parsed.

Polite by construction: one request per second, a browser User-Agent, no cookies, and no
retry storm. `robots.txt` disallows only `/zip/`, `/research/{built,killed,abilities}/` and
`/*/download/` — the search and build pages this tool reads are explicitly allowed. It
honours `robots.txt` anyway rather than assuming the allow-list is stable.

Usage:
    python tools/refresh_build_snapshot.py --out src/lib/builds.json --pages 3
    python tools/refresh_build_snapshot.py --ids 130000 --dry-run
"""

from __future__ import annotations

import argparse
import dataclasses
import html
import json
import pathlib
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BASE_URL = "https://lotv.spawningtool.com"
ROBOTS_URL = f"{BASE_URL}/robots.txt"

# A real browser string, because the site serves Apache logs and a default urllib agent is both
# conspicuous and, on some hosts, rejected outright. It is not a disguise: the request rate and
# the robots.txt check are what make this polite, not the header.
USER_AGENT = (
	"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
	"Chrome/131.0 Safari/537.36 sc2-science-vessel/0.1 (build snapshot; contact via repo)"
)

REQUEST_INTERVAL_SECONDS = 1.0
REQUEST_TIMEOUT_SECONDS = 30.0

# The category classes SpawningTool puts on each step's `<span>`. Anything else is treated as a
# unit rather than dropped, so a new category survives a scrape instead of vanishing silently.
STEP_CATEGORIES = ("Building", "Worker", "Unit", "Upgrade", "Ability")

_ROW = re.compile(r"<tr>(.*?)</tr>", re.DOTALL | re.IGNORECASE)
_CELL = re.compile(r"<td[^>]*>(.*?)</td>", re.DOTALL | re.IGNORECASE)
_TABLE = re.compile(
	r"<table[^>]*class=\"[^\"]*build-table[^\"]*\"[^>]*>(.*?)</table>", re.DOTALL | re.IGNORECASE
)
_STEP_NAME = re.compile(r"<span[^>]*class=\"([^\"]*)\"[^>]*>(.*?)</span>", re.DOTALL | re.IGNORECASE)
_NEXT_PAGE = re.compile(r"<a[^>]*class=\"pull-right\"[^>]*href=\"\?([^\"]*)\"", re.IGNORECASE)
_RESULT_LINK = re.compile(r"<a[^>]*href=\"/build/(\d+)/\"[^>]*>(.*?)</a>", re.DOTALL | re.IGNORECASE)
# Search results are one `<tr>` per build. Rows are split so the matchup token can be read from
# the row holding a result link rather than from anywhere on the page.
_RESULT_ROW = re.compile(r"<tr.*?</tr>", re.DOTALL | re.IGNORECASE)
# `ZvT`, `PvP`, and the `X` variants the site also lists. Normalised to capitalisation on capture.
_MATCHUP = re.compile(r"(?<![A-Za-z])[TPZX]v[TPZX](?![A-Za-z])", re.IGNORECASE)
# The one `<h1>` on a build page, used only as a matchup fallback for builds fetched by id.
_TITLE = re.compile(r"<h1[^>]*>(.*?)</h1>", re.DOTALL | re.IGNORECASE)

_TAG = re.compile(r"<[^>]+>")
_WHITESPACE = re.compile(r"\s+")

# Step times are `M:SS`, or `H:MM:SS` in a game that runs past an hour.
_TIME = re.compile(r"^(?:(\d+):)?(\d+):(\d+)$")


class ScrapeError(RuntimeError):
	"""Raised when a page cannot be understood. Never swallowed: a partial snapshot of silently
	empty builds is worse than an old snapshot, because it looks current."""


def parse_time(text: str) -> float:
	"""Converts SpawningTool's `M:SS` (or `H:MM:SS`) step time to seconds.

	Args:
	    text: The time cell's text, e.g. `"12:04"`.

	Returns:
	    Seconds since the start of the game.

	Raises:
	    ScrapeError: If the text is not a time, because a misread time is a wrong build order
	        and the whole point of the comparison feature is its +/- 10 second tolerance.
	"""
	cleaned = clean_text(text)
	match = _TIME.match(cleaned)
	if not match:
		raise ScrapeError(f"not a step time: {cleaned!r}")
	hours, minutes, seconds = match.groups()
	return (
		(int(hours) * 3600 if hours else 0) + int(minutes) * 60 + int(seconds)
	)


def clean_text(fragment: str) -> str:
	"""Strips tags, decodes entities, and collapses whitespace from an HTML fragment.

	SpawningTool pads every cell with `&nbsp;&nbsp;`, so without this every value would carry
	trailing non-breaking spaces and every name comparison in the app would fail.
	"""
	without_tags = _TAG.sub("", fragment)
	return _WHITESPACE.sub(" ", html.unescape(without_tags).replace("\xa0", " ")).strip()


def parse_build_table(page: str) -> list[dict]:
	"""Extracts the build steps from a build detail page.

	The table is only `<tr>`s: no `<thead>`, no `<tbody>`, no `<th>`, so row 0 is data. Each row
	has exactly four cells — supply, time, name, note — and a row with a different shape is an
	error rather than something to guess at, because that is what a page redesign looks like.

	Args:
	    page: The raw HTML of a build page.

	Returns:
	    Steps of `{supply, second, name, category, note}`.

	Raises:
	    ScrapeError: If no build table is present or a row has an unexpected shape.
	"""
	tables = _TABLE.findall(page)
	if not tables:
		raise ScrapeError("no table.build-table on the page; the layout may have changed")

	steps: list[dict] = []
	for table in tables:
		for row in _ROW.findall(table):
			cells = _CELL.findall(row)
			if len(cells) != 4:
				raise ScrapeError(f"expected 4 cells in a build row, found {len(cells)}: {row[:120]}")
			supply_text, time_text, note_text = (
				clean_text(cells[0]),
				clean_text(cells[1]),
				clean_text(cells[3]),
			)
			# The category lives in the `<span>`'s class, so the *raw* cell must be searched before
			# any tag-stripping — reading the cleaned text finds no span and silently labels every
			# step `Unknown`, which would break the Building/Unit/Worker distinction everywhere.
			name_match = _STEP_NAME.search(cells[2])
			if name_match:
				category, name = name_match.group(1), clean_text(name_match.group(2))
			else:
				# A step with no `<span>` still carries a name; record it as an unknown category
				# rather than dropping the step, since a missing step is invisible in the UI.
				category, name = "Unknown", clean_text(cells[2])
			if not name:
				raise ScrapeError(f"build step without a name: {row[:120]}")
			steps.append(
				{
					"supply": int(supply_text) if supply_text.isdigit() else None,
					"second": parse_time(time_text),
					"name": name,
					"category": category,
					"note": note_text,
				}
			)
	if not steps:
		raise ScrapeError("the build table had no rows")
	return steps


def normalise_matchup(token: str) -> str:
	"""Normalises a matchup token to the site's `RacevRace` spelling.

	`str.capitalize()` is wrong here because the site spells matchups `PvZ` with a lowercase `v`
	and `.capitalize()` would lowercase it into `Pvz`, which would then compare unequal to the
	sidecar's own matchup strings (`ReplayHeader.matchup` builds `PvZ` the same way) and silently
	break every per-matchup filter.

	Args:
	    token: A token such as `zvz`, `PvZ` or `TVT`.

	Returns:
	    The canonical spelling, e.g. `ZvZ`.
	"""
	first, _, second = token.partition("v")
	return f"{first.upper()}v{second.upper()}"


def parse_search_page(page: str) -> tuple[list[dict], bool]:
	"""Extracts the build summaries from a search-results page.

	Reads ids and names from the `/build/<id>/` links rather than from the table layout, so the
	surrounding presentation can change without breaking discovery. The matchup is read from the
	same result row: the detail page's `<h1>` is the *replay's* title, so the matchup token in it
	describes the game the build was used in rather than the build. The search row is the only place
	the site labels a build with its matchup in a field meant for reading.

	Args:
	    page: The raw HTML of a search page.

	Returns:
	    `(summaries, has_more)` where each summary is `{id, name, matchup}` and `has_more` reports
	    whether the page offered a next-page link.
	"""
	seen: dict[int, dict] = {}
	for row in _RESULT_ROW.findall(page):
		# A result row contains two links to the same build: the race/unit thumbnail, whose text
		# is empty, and the title in `<b>`. Taking the first link naively therefore skipped every
		# single row, and the search looked like it had found nothing rather than being broken.
		# The first link that has readable text is the title.
		chosen: tuple[int, str] | None = None
		for id_text, name_fragment in _RESULT_LINK.findall(row):
			name = clean_text(name_fragment)
			if name:
				chosen = (int(id_text), name)
				break
		if chosen is None:
			continue
		build_id, name = chosen
		if build_id in seen:
			continue
		matchup = _MATCHUP.search(row)
		seen[build_id] = {
			"id": build_id,
			"name": name,
			"matchup": normalise_matchup(matchup.group(0)) if matchup else None,
		}
	return list(seen.values()), bool(_NEXT_PAGE.search(page))


def parse_matchup_from_title(title: str) -> str | None:
	"""Reads the matchup token out of a build or replay title.

	A label of last resort: the search page's matchup column is authoritative, but a build fetched
	directly by id has never been seen on a search page and would otherwise be stored with no
	matchup — which silently removes it from every per-matchup comparison.

	Args:
	    title: Text such as `MrFercho v ThikiPizza (PvZ None)`.

	Returns:
	    A normalised matchup such as `PvZ`, or None when the title names none.
	"""
	match = _MATCHUP.search(title)
	return normalise_matchup(match.group(0)) if match else None


@dataclasses.dataclass
class Fetcher:
	"""Fetches pages at a fixed, polite interval, and refuses to fetch disallowed paths.

	Args:
	    interval: Minimum seconds between requests.
	    robots: The site's robots.txt text; paths matching a `Disallow` line are refused.
	"""

	interval: float = REQUEST_INTERVAL_SECONDS
	robots: str = ""
	_last_request: float = dataclasses.field(default=0.0, init=False)

	def _disallowed(self) -> list[re.Pattern[str]]:
		"""Compiles robots.txt's `Disallow` rules into anchored glob patterns.

		`*` is a wildcard anywhere in the path, not just a suffix — SpawningTool disallows
		`/*/download/`, which a naive `startswith` test would reduce to `/download/` and then
		fail to match `/61197/download/`. That mistake turns a rule meant to protect a download
		endpoint into a rule that protects nothing, so the match is done properly here.

		Returns:
		    One compiled pattern per non-empty rule.
		"""
		rules = []
		for line in self.robots.splitlines():
			key, _, value = line.partition(":")
			if key.strip().lower() != "disallow":
				continue
			pattern = value.strip()
			if not pattern:
				continue
			# No `re.escape` on the `*` sentinels: they must become `.*` rather than be escaped.
			parts = [re.escape(part) for part in pattern.split("*")]
			rules.append(re.compile("^" + ".*".join(parts)))
		return rules

	def get(self, path: str) -> str:
		"""Fetches one path, honouring the interval and robots.txt.

		Args:
		    path: A path such as `/build/130000/`.

		Returns:
		    The response body.

		Raises:
		    ScrapeError: On a disallowed path, an HTTP error, or a timeout. The caller decides
		        whether to give up or continue with the rest of the snapshot.
		"""
		for rule in self._disallowed():
			if rule.match(path):
				raise ScrapeError(f"robots.txt disallows {path} (matched {rule.pattern})")

		wait = self.interval - (time.monotonic() - self._last_request)
		if wait > 0:
			time.sleep(wait)
		request = urllib.request.Request(  # noqa: S310 - the scheme is pinned to https by BASE_URL
			f"{BASE_URL}{path}", headers={"User-Agent": USER_AGENT}
		)
		try:
			with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:  # noqa: S310
				body = response.read().decode("utf-8", errors="replace")
		except (urllib.error.URLError, TimeoutError) as error:
			raise ScrapeError(f"failed to fetch {path}: {error}") from error
		finally:
			self._last_request = time.monotonic()
		return body


def load_robots() -> str:
	"""Fetches robots.txt so the fetcher can refuse disallowed paths.

	A failure here is not fatal: the tool then fetches nothing but the search and detail pages
	it was written for, which are allowed. Printing the failure keeps it visible.
	"""
	try:
		request = urllib.request.Request(ROBOTS_URL, headers={"User-Agent": USER_AGENT})  # noqa: S310
		with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:  # noqa: S310
			return response.read().decode("utf-8", errors="replace")
	except (urllib.error.URLError, TimeoutError) as error:
		print(f"warning: could not read robots.txt ({error}); proceeding cautiously", file=sys.stderr)
		return ""


def discover(fetcher: Fetcher, pages: int, matchup: str | None) -> list[dict]:
	"""Walks the search pages and returns the build ids found.

	Args:
	    fetcher: A configured fetcher.
	    pages: How many search pages to walk.
	    matchup: An optional matchup filter such as `ZvT`.

	Returns:
	    Summaries of `{id, name}`, de-duplicated across pages.
	"""
	found: dict[int, dict] = {}
	for page_number in range(1, pages + 1):
		query = {"p": page_number}
		if matchup:
			query["matchup"] = matchup
		path = f"/build/?{urllib.parse.urlencode(query)}"
		try:
			summaries, has_more = parse_search_page(fetcher.get(path))
		except ScrapeError as error:
			print(f"warning: search page {page_number} failed: {error}", file=sys.stderr)
			break
		for summary in summaries:
			found.setdefault(summary["id"], summary)
		print(f"search page {page_number}: {len(summaries)} builds, {len(found)} total", file=sys.stderr)
		if not has_more:
			break
	return list(found.values())


def fetch_builds(fetcher: Fetcher, targets: list[dict]) -> list[dict]:
	"""Fetches and parses each build page.

	A build that fails is reported and skipped rather than aborting the run: one redesigned page
	should not cost the other several hundred, and the caller writes whatever succeeded.

	Args:
	    fetcher: A configured fetcher.
	    targets: Summary records of `{id, name, matchup?}` from `discover`, or bare ids promoted
	        by `main` when the user named them explicitly.

	Returns:
	    Build records of `{id, name, matchup, steps}`.
	"""
	builds: list[dict] = []
	for index, target in enumerate(targets, start=1):
		build_id = int(target["id"])
		try:
			page = fetcher.get(f"/build/{build_id}/")
			steps = parse_build_table(page)
		except ScrapeError as error:
			print(f"warning: build {build_id} failed: {error}", file=sys.stderr)
			continue
		title = _TITLE.search(page)
		matchup = target.get("matchup") or parse_matchup_from_title(clean_text(title.group(1)) if title else "")
		builds.append(
			{
				"id": build_id,
				"name": target.get("name") or f"Build {build_id}",
				"matchup": matchup,
				"steps": steps,
			}
		)
		if index % 25 == 0:
			print(f"  {index}/{len(targets)} builds fetched", file=sys.stderr)
	return builds


def main(argv: list[str] | None = None) -> int:
	"""CLI entry point.

	Returns:
	    0 on success, 1 when nothing could be fetched (so the caller keeps the old snapshot).
	"""
	parser = argparse.ArgumentParser(description="Refresh the bundled SpawningTool build snapshot.")
	parser.add_argument("--out", type=pathlib.Path, help="Write the snapshot here.")
	parser.add_argument("--pages", type=int, default=3, help="Search pages to walk (20 builds each).")
	parser.add_argument("--matchup", help="Restrict the search to one matchup, e.g. ZvT.")
	parser.add_argument("--ids", type=int, nargs="*", help="Fetch these build ids instead of searching.")
	parser.add_argument("--interval", type=float, default=REQUEST_INTERVAL_SECONDS)
	parser.add_argument("--dry-run", action="store_true", help="Print what would be fetched, then stop.")
	args = parser.parse_args(argv)

	fetcher = Fetcher(interval=args.interval, robots=load_robots())

	if args.ids:
		targets = [{"id": build_id, "name": f"Build {build_id}", "matchup": None} for build_id in args.ids]
	else:
		targets = discover(fetcher, args.pages, args.matchup)

	if not targets:
		print("no builds found; keeping the existing snapshot", file=sys.stderr)
		return 1
	print(f"{len(targets)} builds to fetch", file=sys.stderr)

	if args.dry_run:
		for target in targets:
			print(f"  {target['id']}  {target['name']}")
		return 0

	builds = fetch_builds(fetcher, targets)
	if not builds:
		print("nothing fetched; keeping the existing snapshot", file=sys.stderr)
		return 1

	snapshot = {
		"source": BASE_URL,
		"generatedAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
		"builds": builds,
	}
	payload = json.dumps(snapshot, indent="\t", ensure_ascii=False) + "\n"
	if args.out:
		args.out.parent.mkdir(parents=True, exist_ok=True)
		args.out.write_text(payload, encoding="utf-8")
		print(f"wrote {len(builds)} builds to {args.out}", file=sys.stderr)
	else:
		sys.stdout.write(payload)
	return 0


if __name__ == "__main__":
	raise SystemExit(main())

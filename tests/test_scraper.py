"""Tests for the SpawningTool scraper's parsing and its politeness rules.

The parser runs against captured HTML rather than the live site, because a test that needs the
network is a test that fails when the network is down, and this project's whole premise is that
replay analysis works offline. The captured fragment below is the real markup of the step
table — four cells per row, `&nbsp;` padding, the category in the `<span>`'s class — with the
row count trimmed.

What is being protected here is mostly the *failure* behaviour. A scraper that silently returns
an empty or half-parsed build is worse than one that raises, because the app would show a build
order with steps missing and no indication anything went wrong.
"""

from __future__ import annotations

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "tools"))

from refresh_build_snapshot import (  # noqa: E402
	Fetcher,
	ScrapeError,
	clean_text,
	parse_build_table,
	parse_matchup_from_title,
	parse_search_page,
	parse_time,
)

# Verbatim from https://lotv.spawningtool.com/build/130000/ (2026-10), row count trimmed.
BUILD_PAGE = """
<h1>MrFercho v ThikiPizza - King of Braxis Station (PvZ None)</h1>
<table id="build-1" class="build-table" cellpadding="0" cellspacing="0">
<tr><td>&nbsp;&nbsp;12</td><td>&nbsp;&nbsp;0:10</td><td>&nbsp;&nbsp;<nobr><span class="Building">Assimilator</span></nobr></td><td>&nbsp;&nbsp;</td></tr>
<tr><td>&nbsp;&nbsp;12</td><td>&nbsp;&nbsp;0:28</td><td>&nbsp;&nbsp;<nobr><span class="Building">Pylon</span></nobr></td><td>&nbsp;&nbsp;</td></tr>
<tr><td>&nbsp;&nbsp;12</td><td>&nbsp;&nbsp;0:59</td><td>&nbsp;&nbsp;<nobr><span class="Building">Gateway</span></nobr></td><td>&nbsp;&nbsp;</td></tr>
<tr><td>&nbsp;&nbsp;14</td><td>&nbsp;&nbsp;1:09</td><td>&nbsp;&nbsp;<nobr><span class="Worker">Probe</span></nobr></td><td>&nbsp;&nbsp;</td></tr>
<tr><td>&nbsp;&nbsp;18</td><td>&nbsp;&nbsp;2:00</td><td>&nbsp;&nbsp;<nobr><span class="Unit">Stalker</span></nobr></td><td>&nbsp;&nbsp;scout</td></tr>
<tr><td>&nbsp;&nbsp;32</td><td>&nbsp;&nbsp;3:45</td><td>&nbsp;&nbsp;<nobr><span class="Upgrade">WarpGateResearch</span></nobr></td><td>&nbsp;&nbsp;</td></tr>
</table>
"""

ROBOTS = "User-agent: *\nDisallow: /zip/\nDisallow: /research/built/\nDisallow: /*/download/\n"


def test_time_parsing_handles_minutes_and_hours():
	"""`M:SS` and `H:MM:SS` must both convert, since a long game passes one hour."""
	assert parse_time("0:10") == 10
	assert parse_time("12:04") == 724
	assert parse_time("59:59") == 3599
	assert parse_time("1:02:03") == 3723


def test_time_parsing_refuses_anything_that_is_not_a_time():
	"""A misread time is a wrong build order, so it must raise rather than default to zero."""
	with pytest.raises(ScrapeError):
		parse_time("wat")
	with pytest.raises(ScrapeError):
		parse_time("")


def test_clean_text_strips_nbsp_and_tags():
	"""SpawningTool pads every cell with `&nbsp;`; leaving it in breaks every name comparison."""
	assert clean_text("<b>Hello</b>&nbsp;&nbsp;world") == "Hello world"
	assert clean_text("&nbsp;&nbsp;12") == "12"
	assert clean_text("  a\n\t b  ") == "a b"


def test_build_table_parses_four_cells_per_row():
	"""Each row must yield supply, time, name and note from the four expected cells."""
	steps = parse_build_table(BUILD_PAGE)
	assert len(steps) == 6, f"expected 6 steps, got {len(steps)}"
	first = steps[0]
	assert first["supply"] == 12
	assert first["second"] == 10
	assert first["name"] == "Assimilator"
	assert first["category"] == "Building"


def test_build_table_reads_the_category_from_the_raw_span():
	"""The category is a CSS class on a `<span>`, so it must be read before tags are stripped.

	Stripping first leaves no span to find and silently labels every step `Unknown`, which
	destroys the Building/Unit/Worker distinction the whole feature depends on.
	"""
	categories = [step["category"] for step in parse_build_table(BUILD_PAGE)]
	assert categories == ["Building", "Building", "Building", "Worker", "Unit", "Upgrade"], categories
	assert "Unknown" not in categories, "the span lookup is being done after tag-stripping"


def test_build_table_keeps_the_note_and_none_supply():
	"""A note is step metadata and a missing supply must stay `None`, not become 0."""
	steps = parse_build_table(BUILD_PAGE)
	stalker = next(step for step in steps if step["name"] == "Stalker")
	assert stalker["note"] == "scout", stalker
	assert all(step["supply"] is not None for step in steps), (
		"these rows all have supplies; a None here means the supply cell was misread"
	)
	assert parse_build_table(
		'<table class="build-table"><tr><td></td><td>0:05</td>'
		'<td><span class="Unit">Marine</span></td><td></td></tr></table>'
	)[0]["supply"] is None


def test_build_table_rejects_a_page_without_one():
	"""A page with no build table means the layout changed and must raise, not return empty."""
	with pytest.raises(ScrapeError):
		parse_build_table("<html><body><p>Nothing here</p></body></html>")


def test_build_table_rejects_a_row_of_the_wrong_shape():
	"""A row with three cells is a redesigned table; guessing would mis-assign the columns."""
	broken = (
		'<table class="build-table"><tr><td>12</td><td>0:10</td>'
		"<td>Pylon</td></tr></table>"
	)
	with pytest.raises(ScrapeError):
		parse_build_table(broken)


def test_search_page_reads_ids_names_and_matchups():
	"""Discovery reads `/build/<id>/` links, and the matchup comes from the same result row.

	The row is the only place SpawningTool labels a build with its matchup for reading; the detail
	page's `<h1>` is the replay's title, so a build fetched straight from search would otherwise
	be stored with no matchup and drop out of every per-matchup comparison.
	"""
	page = (
		'<table class="table">'
		'<tr><td><img alt="Protoss"/></td>'
		'<td><b><a href="/build/203087/">2gate</a></b></td>'
		"<td>PvZ</td><td>Economic<br/>Easy</td></tr>"
		'<tr><td><img alt="Zerg"/></td>'
		'<td><b><a href="/build/130000/">Ling Bane All-In</a></b></td>'
		"<td>ZvT</td><td>All-In<br/>Hard</td></tr>"
		"</table>"
		'<p>Page 1 of 21</p><a class="pull-right" href="?&p=2">Next</a>'
	)
	summaries, has_more = parse_search_page(page)
	assert [summary["id"] for summary in summaries] == [203087, 130000]
	assert summaries[0]["name"] == "2gate"
	assert summaries[0]["matchup"] == "PvZ"
	assert summaries[1]["matchup"] == "ZvT"
	assert has_more is True

	last_page = '<a href="/build/1/">Last</a><p>Page 21 of 21</p>'
	summaries, has_more = parse_search_page(last_page)
	assert has_more is False


def test_search_page_reads_a_lowercase_matchup_token():
	"""The site renders matchups inconsistently; the token must survive either case.

	`str.capitalize()` would spell this `Zvz`, which compares unequal to the sidecar's own `ZvZ`
	and would silently drop the build out of a per-matchup filter.
	"""
	page = '<tr><td><a href="/build/5/">Pool first</a></td><td>zvz</td></tr>'
	summaries, _ = parse_search_page(page)
	assert summaries[0]["matchup"] == "ZvZ"


def test_search_page_skips_the_thumbnail_link_in_each_row():
	"""A result row links its build twice: a race thumbnail with no text, then the title.

	Reading only the first link skipped every row in the real page, so a working search looked
	identical to an empty one. The markup below is the shape taken from
	`https://lotv.spawningtool.com/build/?matchup=ZvT` (2026-10).
	"""
	page = (
		"<tr><td><a href=\"/build/203208/\"><img src=\"/static/core/images/zerg.png\" alt=\"Zerg\"/></a></td>"
		"<td><b><a href=\"/build/203208/\">3 Roach Opener - 8 Worker Eco - ZvT</a></b>"
		"&nbsp;&nbsp;<br /><small>Created by <a href=\"?contributor=3922&sort_by=r\">railgan</a></small></td>"
		"<td>ZvT</td><td>Economic<br />Easy</td></tr>"
	)
	summaries, _ = parse_search_page(page)
	assert len(summaries) == 1
	assert summaries[0]["id"] == 203208
	assert summaries[0]["name"] == "3 Roach Opener - 8 Worker Eco - ZvT"
	assert summaries[0]["matchup"] == "ZvT"


def test_matchup_token_is_not_taken_from_a_longer_word():
	"""A word that merely contains `ZvT` must not be read as a matchup."""
	page = '<tr><td><a href="/build/6/">Avzta build</a></td><td>Zerg</td></tr>'
	summaries, _ = parse_search_page(page)
	assert summaries[0]["matchup"] is None


def test_matchup_falls_back_to_the_page_title_for_a_direct_id_fetch():
	"""A build fetched by id was never on a search page, so its title is the only label."""
	assert parse_matchup_from_title("MrFercho v ThikiPizza v tigro (PvZ None)") == "PvZ"
	assert parse_matchup_from_title("A game with no token") is None


def test_robots_rules_match_wildcards_anywhere_in_the_path():
	"""`/*/download/` must block `/61197/download/`.

	Treating `*` as a removable suffix reduces the rule to `/download/`, which then matches
	nothing — a rule meant to protect a download endpoint that protects nothing.
	"""
	fetcher = Fetcher(interval=0.0, robots=ROBOTS)
	rules = fetcher._disallowed()
	assert any(rule.match("/61197/download/") for rule in rules), "mid-path wildcard not honoured"
	assert any(rule.match("/zip/x") for rule in rules)
	assert any(rule.match("/research/built/x") for rule in rules)
	for allowed in ("/build/130000/", "/build/?p=2", "/build/zvt/"):
		assert not any(rule.match(allowed) for rule in rules), f"{allowed} is allowed by robots.txt"


def test_a_disallowed_path_is_refused_before_any_request():
	"""The refusal must happen without touching the network, so it cannot be a 403 check."""
	fetcher = Fetcher(interval=0.0, robots=ROBOTS)
	with pytest.raises(ScrapeError):
		fetcher.get("/61197/download/")


def test_an_empty_robots_file_blocks_nothing():
	"""A robots.txt fetch failure must not brick the scraper, only stay visible as a warning."""
	fetcher = Fetcher(interval=0.0, robots="")
	assert fetcher._disallowed() == []

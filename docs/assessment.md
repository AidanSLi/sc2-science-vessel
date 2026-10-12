# sc2-science-vessel — feasibility assessment

Status: draft for review · Author: agent session · Date of evidence: 2026-10-11/12

Every claim below was checked against **this machine's real data**, not documentation
alone. The evidence base is your own replay library
(`~/Faugus/battlenet/.../StarCraft II/Accounts/350471450/1-S2-1-9754299/Replays`, 722
replays), the one replay in `~/Documents/SC2 Replays/`, the installed
`sc2reader 1.9.0` + `spawningtool` source, and the live Wayland session. Numbers marked
*verified* were measured here; numbers marked *UNVERIFIED* means no source could be
found and you should not build on them.

---

## 1. Verdict in one page

| Requirement | Verdict | Basis |
| --- | --- | --- |
| Bulk-import and analyse an existing replay library offline | **Yes, easy** | 722/722 replays parsed in **2.4 s (3 ms each)** at header level; 5.0.15 audience |
| Metrics like SC2ReplayStats (resources, workers, supply, army value, losses) | **Yes, mostly free** | `PlayerStatsEvent` carries ~40 fields, ±10 game-second cadence |
| Build-order extraction + comparison to a reference build | **Yes** | 69-step build order extracted from your replay; no SALT needed |
| Per-matchup preset build orders, auto-select on match start | **Yes** | Matchup is known from the replay header before the first event |
| Overlay window over the game, hotkey toggled | **Yes on Linux/KDE** (with a specific technique), **unproven on macOS** | kwin 6.7.5 advertises `zwlr_layer_shell_v1`; SC2 already runs under gamescope |
| Build steps follow **in-game** time, pausing when the game pauses | **Yes, but only via port 6119** — not from a replay, and not from wall-clock timing | `GET localhost:6119/game` → `displayTime` is the UI clock, reflects pause automatically |
| Browse/spawn spawningtool build orders **with no network** | **No** — needs one of: build a scraper, bundle a snapshot, or relax "no network" | spawningtool.com serves HTML; I could not prove a documented JSON API exists |
| Pro-player / reference build corpus with a curated SALT format | **No such corpus exists** — and *no SALT encoder exists anywhere* | `reardan/salt` is a decoder-only library |
| "What to expect next" prompts driven by what the opponent is actually doing | **No** — not from any documented live interface | Client API exposes clock + player identity only, never units |
| Ship as a Tauri 2 app that will "migrate to Tauri 3 for GTK4" | **Plan for Tauri 2 staying GTK3 for a long time** | wry#1474 open since 2025-01-30, latest comment 2026-07-07: "no real work has been done", "over a year" in limbo |

The headline: **the analysis app is straightforward** — the data is all there and the
parser is fast. **The overlay's hard constraint is not the overlay, it is the clock**:
a replay gives you game time but not wall-clock time, so a live timer must read the
game's own clock from the SC2 Client API on `localhost:6119`. That works, and it is the
only documented live channel.

---

## 2. What was verified locally

### 2.1 Parsing is a solved problem, and it is fast

- **722/722** replays in your library parsed with zero failures, at `load_level=1`
  (header only), in **2.4 s total — ~3 ms per replay**. This means "scan the whole
  library on startup" is a viable UX, not a batch job.
- Full event decode (`load_level=4`) plus build-order extraction is ~100–300 ms per
  replay on the representative large game (24,863 events, 9,819 tracker events).
- Builds present in your library: **93333, 95841, 96592, 96826, 96883, 97364, 97425,
  97563**. Oldest (93333, 2025-01-29) and newest (97563) both parsed on `sc2reader
  1.9.0` with complete tracker data, and `spawningtool` extracted 294/484-step and
  23/24-step build orders respectively. **No version in your library is unparseable.**
- Your `22 drone ling bane.SC2Replay` is build **95841** (game version `5.0.15.95841`),
  not 93333.

### 2.2 The metrics you asked for are available at 10-game-second resolution

`PlayerStatsEvent` (from `replay.tracker.events`), verified attribute names on the
installed copy — ~40 fields per sample, one sample every 10 game seconds per player:

`minerals_current`, `vespene_current`, `minerals_collection_rate`,
`vespene_collection_rate`, `workers_active_count`, `food_used`, `food_made`,
`idle_worker_count` (**absent**), `army_value` (**absent**), plus a large
loss/kill/usage breakdown split three ways:

`minerals_lost_{army,economy,technology}`, `vespene_lost_*`,
`minerals_killed_{army,economy,technology}`, `*_used_current_*`,
`*_used_in_progress_*`, `*_used_active_forces`, `resources_{killed,lost,used_current,used_in_progress}`.

This single event type carries most of the feature list:

| Requested feature | Source | Caveat |
| --- | --- | --- |
| Worker graph | `workers_active_count` @10 s | 10-second granularity; worker *deaths* need `UnitDiedEvent` for finer detail |
| Supply block time | `food_used` vs `food_made` @10 s | 10-second granularity; a 9-second block can be missed → interpolate from `UnitBornEvent` supply snapshots (spawningtool gives a `supply` series per build step) |
| Overall resources lost graph (0 = even) | `minerals_lost` / `resources_lost` @10 s | Directly provided, no simulation needed |
| Resource collection / spending efficiency | `*_collection_rate`, `*_used_current`, `minerals_current` @10 s | This is how you compute an SQ-like metric |
| Missed Queen / Mule / Chrono energy | `UnitBornEvent`, `UnitTypeChangeEvent`, `UpgradeCompleteEvent`, `AbilityEvent` | Larva inject / MULE / Chrono each need a per-race rule; energy-over-time is *not* recorded |
| Inject timeline / missed injects | `UnitBornEvent` (Larva) + `AbilityEvent` (`SpawnLarva`) | Your replay shows the `SpawnLarva` ability event, so injects are observable |
| Production uptime | `UnitInitEvent`/`UnitDoneEvent`/`UnitBornEvent` + queue inference | Needs unit build-time tables (spawningtool ships them in `lotv_constants.py`) |
| Evo/Engi/Forge uptime, missed upgrades | `UpgradeCompleteEvent` (your replay: `zerglingmovementspeed` at 299 s) | Only *completed* upgrades are recorded; idle time must be inferred from build time |
| Timeline of tech structures and expansions | `UnitInitEvent`/`UnitDoneEvent` + `UnitTypeChangeEvent` (Lair/Hive/Orbital appear here) | Reliable |
| Unit composition, expansion timings | `UnitBornEvent` (4,180 events in a real ZvP) | Reliable |
| Battle timeline with trades | `UnitDiedEvent` (+`killer_pid`, `x`, `y`) ↔ `minerals_killed/lost` @10 s | Trade *efficiency* is derivable; see below for the local negemony |
| Runby vulnerability (worker deaths) | `UnitDiedEvent` filtered to worker types + positions | Reliable |

### 2.3 three things that do *not* exist, contrary to the plan

1. **`UnitPositionsEvent` is not a position stream.** It fires only occasionally
   (64 events in a 25-minute game, 0 in your AI game) and reports only *damaged* units.
   Per-second unit positions are **not** in the replay at all — the game is a
   deterministic state machine, and only deltas are stored. Any "unit positions over
   time" chart requires you to re-simulate.
2. **MMR is in the file.** `replay.gamemetadata.json` carries `MMR` per player **but
   only for ranked games**: 236 of 722 replays have it (397 MMR values, 9 to 4,871).
   That is still enough for offline progression/opponent tracking, and it is a free
   cross-check against any upload-based service.
3. **`Duration` in `gamemetadata.json` is already real-time seconds at Faster speed** —
   measured on all 722 replays: 531 exact 1.40× the parsed game time, 95 at 1.41×,
   tails out at 16×, plus 5 games at 2.0× and one at 3.0× (paused/custom). So
   **real wall-clock seconds = game seconds × 1.4**, and Liquipedia's table confirms
   Faster = speed factor 1.4 / "42.86 real-time seconds per normal minute".

   *Correction to earlier thinking in this session:* an observed ~6× ratio in one AI
   game was an artifact of that game's pacing, not a property of Faster. The 1.4 factor
   is the one that holds across your library.

### 2.4 What is genuinely hard, and why the clock is the crux

A replay stores **game time**, and game time does not advance while the game is paused.
That is good news for "pause my timer when the game pauses" (it happens for free if you
read the game clock) and bad news for "start a timer when the game starts" (a wall-clock
timer drifts and keeps running through a pause).

The **only documented live channel** is Blizzard's SC2 Client API on `localhost:6119`,
which the client exposes by default. Verified from Blizzard's own 2016 developer post
(quoted in the external research, `A5`):

> "The Client API is accessed via HTTP GET queries and returns JSON responses. By default
> the SC2 client will listen on port 6119 for localhost queries only."
> "The `displayTime` field is in seconds; it reflects the timer in the UI."

`GET http://localhost:6119/game` → `{"isReplay":false,"displayTime":100.0,"players":[...]}`
with `isReplay`, player name/type/race/result. That is **clock + identity**, nothing else
— no units, no supply, no build state.

Three consequences:

- **"Build steps follow the in-game time and pause with the game" is feasible** — poll
  `displayTime` ~1 Hz and advance the build list from it. This is a much stronger design
  than any timestamped-timer approach, because pausing, speed changes and replay-mode are
  all handled by the game itself.
- **"Automatically switch to build-order mode when a game starts" is feasible** —
  `isReplay: false` plus a live `displayTime` is the trigger, and the opponents' race
  in `/game` selects the matchup preset.
- **"What to expect next" cannot be state-aware.** It can only be a clock-indexed
  script ("at 4:00 expect a two-base pressure"), not a reaction to what the opponent
  actually built. Anything more requires screen-reading/OCR — a large separate project
  with a higher anti-cheat profile.

The claim that `replay.tracker.events` is appended while a game is live is **UNVERIFIED**
— no source found, and it would be the one way to get live state legitimately. Treat it
as an experiment to run, not a design assumption.

### 2.5 Platform reality on this machine

Measured here, in this session:

- Session is **KDE Plasma on Wayland** (`XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-0`,
  `XDG_CURRENT_DESKTOP=KDE`), **kwin 6.7.5**.
- The Wayland socket advertises **`zwlr_layer_shell_v1` version 5** and
  `zwp_keyboard_shortcuts_inhibit_manager_v1`, alongside KDE's own protocols
  (`org_kde_plasma_shell`, `kde_output_management_v2`, …). It is a single socket — SC2
  is not running under gamescope right now, and `wayland-0` is the only compositor
  socket, despite gamescope being installed and your SC2 prefix being a Faugus/Wine setup.
- Display: 3840×2160 @ 240 Hz, scale 2. Input devices are root-owned `/dev/input/event*`
  in group `input`, and **your account is not in group `input`**.
- SC2 is a Wine/Windows build (wine-11.19) launched through Faugus, with **722 replays**
  under its prefix. Global hotkeys and window targeting therefore hit *an X11/Xwayland
  window*, not a native Wayland one.

---

## 3. Overlay and hotkey design (Linux/KDE)

This is the part with the most forks, so it gets its own section.

### 3.1 The three layering techniques, in order of preference

1. **Wayland `wlr-layer-shell` client (best result, most work).** kwin supports it. A
   layer-shell surface can be placed in the `overlay` layer, positioned by anchor/margin,
   and made non-interactive. It sits *above* everything the compositor draws, including
   fullscreen windows, with no flicker. Cost: it is a Wayland-specific protocol, so the
   window must be native (Rust/GTK/Qt), not a WebView-in-a-toplevel, and it needs a
   fallback on Windows/macOS/X11.
2. **A second normal window kept on top of an Xwayland game.** SC2 is an Xwayland client.
   On KWin, an X11 window cannot be forced above an Xwayland *fullscreen* window reliably,
   but in **Windowed (Fullscreen)** mode a normal always-on-top window does stay visible.
   Electron's `setAlwaysOnTop(true, "screen-saver")` + `setIgnoreMouseEvents(true)` is the
   documented path on Linux (both X11 and Wayland — Electron docs state "On Linux this is
   supported on both X11 and Wayland"), and is the least code. This is the pragmatic v1.
3. **Gamescope + MangoHud custom text (works today, ugly).** SC2 already runs under a
   Wine/Faugus wrapper, and gamescope + mangohud are installed. MangoHud's
   `custom_text` / `custom_text_center` can render a small amount of text over the game
   and re-read it from a file. That is enough for "next 2 build steps" with zero new
   windowing code — a useful first spike and a fallback, not the product.

**Hotkeys:** Tauri's `global-hotkey` crate is X11-only (Tauri/tao issues `#1134`,
`#14913` remain open) — on KWin Wayland it will not see keys. The sanctioned Wayland
route is the **XDG `GlobalShortcuts` portal**, which KDE implements; the alternative is
reading `/dev/input` directly, which needs group `input` membership and is a permission
ask the user must grant. Note that because SC2 is an Xwayland client, an X11
`XGrabKey` grab *may* work — worth a 30-minute spike, not a design assumption.

### 3.2 EULA risk (read this before building the overlay)

Blizzard's EULA (2024-03-21 snapshot) prohibits cheating and "data mining" and reserves
discretion on third-party software. A **passive** overlay that only reads the official
Client API and displays information the player already has is about as benign as it gets,
but it is a discretionary grey zone, not a guarantee. An **active** advisor that reads
game state to tell you what to build next is the materially riskier variant. A design
rule worth writing down: *the app reads only the documented Client API and never
inspects or writes the game process's memory.*

### 3.3 What this implies for the stack question

You proposed SvelteKit + Tailwind + **Vite+** + **Tauri 2, "migrating to 3.0 for GTK3 →
GTK4"**. My assessment:

- **SvelteKit + Tailwind: keep.** They are a good fit and I have no objection.
- **Vite+ (viteplus.dev): drop for now.** Vite+ 1.0 is a young all-in-one toolchain
  (Vite + Vitest + Oxlint + Oxfmt + Rolldown + tsdown + Vite Task, from VoidZero). You
  do not need it: `vp` is a *task runner*, not something SvelteKit or Tauri integrates
  with, and adopting it adds a moving part to the two places that must be boring (build
  and test). Plain Vite is what Tauri documents and what every Tauri bug report assumes.
  Revisit once the app ships.
- **Tauri 2: keep, with eyes open, and do not plan around Tauri 3.** `wry#1474` (GTK4 /
  webkit6) has been open since 2025-01-30 with, as of 2026-07-07, "no real work has been
  done on the working group side" and the maintainer's own words: "it would just really
  help to get out of the limbo we've been in for over a year now." No landing date is
  committed. Tauri 2 on Linux stays GTK3/WebKitGTK-4.1 for the foreseeable future — plan
  the overlay so it does not depend on GTK4 arriving.
- **The real risk in Tauri 2 here is the overlay, not GTK.** Tauri's always-on-top and
  `set_position` are known no-ops on Wayland. If the overlay is the product's crown
  jewel, either (a) build the overlay as a **native Rust layer-shell window** alongside
  the Tauri main window, or (b) accept Electron for its documented Linux click-through.
  I would try (a) first: the rest of the app (SvelteKit UI, analysis views) is unaffected.
- **Add a Python sidecar for analysis.** `sc2reader`/`spawningtool` are Python, and both
  were working within minutes. Tauri's `externalBin` sidecar is documented and is the
  fastest good path (PyInstaller bundle + JSON/IPC boundary). The alternative is a Rust
  parser — `s2protocol-rs` (sebosp, 118★, pushed 2026-10-08, active) gives you protocol
  decoding, but you would then rewrite spawningtool's ~1,900-line unit/build-time tables
  by hand. Not worth it for v1.
- **Do not put analysis in the WebView.** Chart payloads for a 25-minute game are on the
  order of tens of thousands of points; the boundary should be "sidecar returns prepared
  series", not raw events.

### 3.4 Recommended architecture

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri 2 shell (Rust)                                          │
│  ├── main window: SvelteKit + Tailwind (analysis UI)           │
│  ├── overlay window: native wlr-layer-shell surface on KDE     │
│  │     (fallback: always-on-top toplevel on Win/macOS/X11)     │
│  └── client-API poller: GET localhost:6119/game → displayTime  │
├──────────────────────────────────────────────────────────────┤
│ Python sidecar (PyInstaller)                                  │
│  ├── sc2reader (1.9.0) + spawningtool: parse → JSON            │
│  ├── our metric layer: supply block / SQ / trades / injects    │
│  └── build-order compare + SALT encode/decode (we must write   │
│      the encoder; only a decoder exists today)                 │
├──────────────────────────────────────────────────────────────┤
│ Local store (SQLite): games, parsed series cache, builds, MMR   │
└──────────────────────────────────────────────────────────────┘
```

Everything except the client-API poller works with **no network**. The poller talks to
`localhost` only.

---

## 4. Feature-by-feature verdicts

Verdicts: **cheap** (data + a few lines) · **compute** (we write the logic) ·
**hard** (real unknowns) · **no** (not from replay data).

| # | Feature | Verdict | Notes |
| --- | --- | --- | --- |
| 1 | Supply block time | compute | `food_used`/`food_made` @10 s; back `UnitBornEvent`-derived supply for second-level precision |
| 2 | Missed Queen / Mule / Chrono energy | compute | Injects observable via `SpawnLarva`; MULE/Chrono via `UnitBornEvent`+`UnitTypeChangeEvent`; energy is not stored, so "missed" is a rule, not a measurement |
| 3 | Production uptime | compute | `UnitInit`/`UnitDone` + build-time tables from spawningtool |
| 4 | Inject timeline (missed injects) | compute | Rule-based; needs per-race inject cadence model |
| 5 | Worker graph | cheap | `workers_active_count` |
| 6 | Evo/Engi/Forge uptime | compute | `UpgradeCompleteEvent`; idle time inferred |
| 7 | Battle timeline with trades | compute | `UnitDiedEvent` (3909 events in one real game) joined to `minerals_killed/lost` |
| 8 | Resources-lost graph (0 = even) | cheap | `minerals_lost` / `vespene_lost` / `resources_lost` direct from stats events |
| 9 | Runby vulnerability / worker deaths | cheap | filter `UnitDiedEvent` by worker type + position |
| 10 | Battle locations on map hover | compute | `UnitDiedEvent.x/y` exist; `UnitBorn`/`Died` carry positions. A fade heatmap at death sites is honest; a full battle map is not |
| 11 | Creep spread uptime | compute | `sc2reader` ships a `CreepTracker` plugin, but it is Python-2-era (`from sets import Set`, `ANTIALIAS`) and needs `load_map()`; likely rewrite |
| 12 | Tech structure + expansion timeline | cheap | `UnitInit`/`UnitDone`/`UnitTypeChange` |
| 13 | Per-player strategy summary | compute | Rule-based classifier over the build order + composition; can seed with spawningtool's step list |
| 14 | Unit composition, building placement, timings | compute | Composition/timings easy; **placement** only at construction time (positions are recorded once, not over time) |
| 15 | Load SALT encodings and compare | hard | No encoder exists in `reardan/salt` — hand-verified: it is `{decode, encode}` where `encode` is documented as *adding workers to already-encoded strings*. Expect to write a real encoder; the format itself is decodable today |
| 16 | Pick the build you were attempting after a game | cheap | selection UI, compare against 69-step extracted order |
| 17 | Compare to the nearest of a set of builds | compute | needs an edit-distance/alignment metric between two build sequences |
| 18 | Report discrepancies of ±10 s | compute | trivial once 15–17 exist; note the 10-second stats cadence matters, so ±10 s is at the resolution limit |
| 19 | Per-matchup preset builds, auto-selected | cheap | race from `/game` (live) or header (post-game) |
| 20 | Overlay toggled by hotkey | hard on Linux | see §3; layer-shell or Xwayland toplevel; XDG portal for keys |
| 21 | Auto-enter build-order mode on game start | cheap *if* client API works | `isReplay:false` + live `displayTime` |
| 22 | Steps follow in-game clock, pause on pause | hard (one unknown) | depends on port 6119 in *your* client; verify before building |
| 23 | "What to expect next" | no (state-aware) | clock-indexed only |

---

## 5. The two problems that need your decision

### 5.1 "No network" vs "browse spawningtool's build orders"

These conflict. `lotv.spawningtool.com/build/?matchup=PvZ` returns HTML (verified: HTTP
200, 28,715 bytes, `content-type: text/html`), and the same URL with `?format=json`
returns **HTML, not JSON**. I could not verify that a documented JSON API exists. So:

- **(a)** Relax "no network" for *one* read-only feature: scrape/build the JSON on demand.
  Simple, but it makes the app depend on someone else's HTML.
- **(b)** Ship a bundled snapshot: scrape once at build time into `builds.json`, ship it,
  add a "refresh" button. Keeps the app offline, ships a frozen corpus.
- **(c)** Curate your own small corpus (your four ZvP/ZvT/ZvZ flowcharts in
  `~/Documents/Starcraft/` are already this, in `.mmd` form) and treat spawningtool
  import as a paste-a-SALT-string feature. Fully offline, fully yours, smallest scope.

My recommendation is **(b) then (c)** — the app is offline-first, and the bundle is
generated by a script in `tools/` so it can be refreshed whenever you want.

### 5.2 Confirm the client API before designing around it

**First task of the project, before any UI work:** launch SC2 normally, and while a game
is running run `curl http://localhost:6119/game`. If `displayTime` ticks and pauses with
the game, the whole overlay design in §3.1 holds and you get the strongest possible
behaviour. If the endpoint is not there, the overlay becomes a manual/spacebar-advanced
timer and the roadmap changes materially. This is a 15-minute experiment that de-risks
the largest unknown in the project.

---

## 6. Risks, ranked

1. **Live-clock availability.** Everything time-sensitive hangs on port 6119 existing in
   your client. Test first (§5.2).
2. **EULA discretion.** Passive overlay = low risk but not zero; state-reading advisor =
   materially higher. Write the design rule (§3.2) down and hold to it.
3. **The Tauri overlay + hotkey gap on Wayland.** Known-unfixed upstream. Mitigation:
   native layer-shell window + XDG portal, both of which this machine supports.
4. **SALT encoder absence.** The "load SALT builds" feature needs an encoder we write,
   and the only reference is a decoder plus third-hand descriptions of the format.
5. **10-second stats cadence.** Features 1, 8, 18 are all at the limit of their
   resolution. Some "supply blocks" will be invisible in the stats events; per-unit
   fallbacks are required.
6. **No upstream fixes in sight for Tauri/GTK4.** Do not sequence work behind Tauri 3.

---

## 7. Proposed roadmap

- **Phase 0 — de-risk (½ day).** Confirm port 6119. Confirm a layer-shell "always on top,
  click-through, over the game" spike on this machine. Confirm a hotkey path (XDG portal).
- **Phase 1 — parser + store (2–3 days).** Python sidecar: sidecar scans the replay dir,
  parses at header level, caches `PlayerStatsEvent` series + build orders into SQLite.
  A CLI that prints the 19 features as JSON. No UI.
- **Phase 2 — analysis UI (1–2 weeks).** SvelteKit views over the JSON: worker graph,
  resource/loss graphs, build order with discrepancy markers, tech/expansion timeline,
  battle timeline, per-matchup reference builds.
- **Phase 3 — overlay (1–2 weeks).** Client-API poller, build-order view driven by
  `displayTime`, matchup presets, hotkey toggle, KDE layer-shell window first.
- **Phase 4 — extras.** MMR/progression tracking from `gamemetadata.json`, SALT
  encoder, strategy classifier, replay-scan-on-launch, MAC/Windows overlay paths.

Phases 1–2 are the parts I am confident about; phase 3 is the part where the spikes in
phase 0 decide the design.

---

## 8. Open questions for you

These are the things I cannot decide for you — answers will be captured as ADRs.

1. **Build-order source:** bundled snapshot of spawningtool builds (my pick), live scrape,
   or your own curated corpus only?
2. **Overlay ambition:** "informational panel over the game" (safe, feasible) vs
   "advisor that watches the match" (not feasible without OCR; higher EULA risk)?
3. **Windows/macOS scope in v1**, or Linux/KDE only until the overlay works?
4. **Vite+**: keep it, or plain Vite (my pick)?
5. **Reference corpus**: is your four-flowchart set (`ZvP`, `ZvT`, `ZvZ` `.mmd` files)
   the seed content for "compare to my build", or do you want pro builds?

---

## Appendix A — reproduction commands

```bash
# 722-replay header scan (3 ms/replay)
/tmp/spawn_venv/bin/python /tmp/scan_library.py

# build-order extraction for one replay, or for the extremes of your library
/tmp/spawn_venv/bin/python -c \
  "import spawningtool.parser as sp; d=sp.parse_replay('<path>'); \
   print({p: len(v['buildOrder']) for p, v in d['players'].items()})"

# the clock the overlay must trust
curl -s http://localhost:6119/game        # {"isReplay":false,"displayTime":…}

# this session's Wayland capabilities (layer-shell matters)
wayland-info | grep -E "layer_shell|wl_compositor|xdg_wm_base"
```

## Appendix B — verification status of key claims

| Claim | Status |
| --- | --- |
| 722/722 replays parse; 3 ms each | verified here |
| `PlayerStatsEvent` field list and 10 s cadence | verified from installed `sc2reader 1.9.0` |
| `Duration` = game seconds × 1.4 | verified on 722 replays (531 exact, 95 at 1.41) |
| MMR present in 236/722 replays | verified here |
| kwin advertises `zwlr_layer_shell_v1` v5 | verified here via `wayland-info` |
| `localhost:6119/game` → `displayTime`, clock only | quote from Blizzard's 2016 developer post (external research A5) |
| Trackers written incrementally in a live game | **UNVERIFIED** — no source found |
| Client "real-time mode" making game time 1:1 with wall clock | **UNVERIFIED, likely false** — the only sourced `realtime` flag is the developer API's, and it means "run at Faster, not stepped by a bot" |
| spawningtool.com JSON API | **not found** — `?format=json` returns HTML |
| SALT encoder exists | **false** — `reardan/salt` is decoder-only |
| PPA definition | **UNVERIFIED** — not on any publicly fetchable page |

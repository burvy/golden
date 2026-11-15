# Library lab: cheap gates that cut candidates without losing the best spots

## Headline numbers

- **15 cost functions measured and kept**, all 15 in the search today.
- **Recall kept: 100%.** Every one keeps all of the search's top 10 spots in every test: 18 places in
  downtown LA and Princeton, 3 days (Nov 14, Jun 21, Dec 21), up to 100 runs with spots per entry.
- **They skip half the work or more.** 12 of them have a cheap check that drops a median 41-93% of
  candidate standpoints before the skyline trace (the other 3 get theirs with task 065).
  Example: "the sun setting down a street" traces 55,248 -> 27,282 skylines; search 1283 -> 626 ms.
- **Shots covered, sunset and sunrise:** sun alone (open horizon, wide gap, down the street, keyhole),
  one tower (sun beside or behind it, golden light on it, looking up at it), two towers (sun between
  them, the skyline glowing).

Task 062. The goal is a growing library of cost functions that drop candidates cheaply, before the
skyline trace, without losing the spots an ungated search ranks best. Recall is the guardrail and
prune % is the win. Each kept entry is one solved class of shot (docs/metrics.md v2).

## Run it

```
$env:VEXCEL_CACHE_DIR = "Z:/3Projects/golden-cache"     # cached height maps; nothing is fetched
$env:COSTAR_KIT_DIR   = "<unzipped CoStar kit>"         # optional: LA buildings as objects
cargo run -p golden-core --release --example library_lab -- [--write | --report] [--place NAME] [--scenario KEY] [--gate NAME] [--radius M]
```

`--write` rewrites the generated section below and saves the raw rows to `cache/library_lab/rows.json`.
Places run in parallel, 6 at a time (`LAB_THREADS=N` to change; searches are single-threaded). A full run
takes about 12 minutes with `LAB_THREADS=9` (18 places x 3 days x 28 scenarios). Times in the tables
are measured under that load, so compare them within a row, not across rounds. `--report` rebuilds the
generated section from the last `--write`'s saved rows (`cache/library_lab/{rows,shots,places}.json`)
without searching.
`--radius M` runs with another area radius (default 300 m; printed only, never written). A place
whose tiles for that radius aren't all cached is skipped. `LAB_DEBUG=1` prints each search's funnel
stages and picked spots with their scores (that's how the round 2 brief bug below was found).

## How it measures

For each place, day and scenario, search runs once **ungated**, then once per gate with the gate
added as a `must`. A must goal has weight 0, so a gate never changes the score of a candidate it
keeps. That makes recall exact: if every ungated top spot survives the gate, the gated search picks
exactly the same spots (picking is greedy best-first, so dropping unpicked candidates changes
nothing).

- **prune**: share of the heat-grid candidates the gate drops (from a gate-only search). In the sun
  scenarios every dropped candidate is one skyline trace saved, because gates run first and the
  skyline is only traced when a Skyline feature is asked for.
- **recall@10**: share of the ungated top-10 spots still picked with the gate.
- **near-best kept**: ungated candidates scoring at least 0.8 of the best that survive.
- **time**: wall clock of the ungated and the gated search (one run each, so a few % of noise).

An entry is **kept** when recall@10 is 100% in every run, it ran on at least 2 places, and its median
prune is at least 30%.

**Objects.** CoStar buildings where the kit has them (LA; their top is measured on the height map,
within 20 m of the kit's point). Princeton has no CoStar data, so there the lab finds buildings in the
height map itself: 25 m blocks whose top stands at least 8 m above the street and above the
neighbouring blocks. "The tower" is the tallest building object inside the area.

**Two classes of gate** (Boss, task 062). **(a) Speed gates** are implied by a goal's score, so
dropping a point changes only the time. They're judged by recall@10 = 100% and prune. **(b) Shot
classes** (e.g. "on a roof") are the user's intent and change the result on purpose. They're judged
by prune and a sane result, not by recall. A library entry can be a shot class whose own `must` holds
its speed gate (`sunset_beside`). Its recall is measured against the same scenario with that `must`
removed: the lab swaps the entry in for the plain goal, keeping that goal's weight.

**Gate kinds.** *entry*: a cost function in `library/*.json`, reusable by any brief. *sweep*: the
same idea at a few thresholds (or a constraint we tried and didn't save). *what-if*: needs a number
fixed per brief (the day's sun azimuths), which no Point feature gives yet. It shows what such a
feature would buy and isn't saved. *ray what-if*: needs a ray march no feature offers yet. It's
evaluated in the lab with core's public `horizon::ray_max_below` at every candidate and every
ungated top spot (exact recall, since a must never rescales survivors). Its traces and time are
estimates: the base time scaled by the candidates left, plus the rays' own time.

**Skyline traces** are counted from each search's funnel `stages`: the candidates left when the funnel
reaches its first Skyline-tier gate, or all survivors if no gate is Skyline-tier (every scenario
ranks on a sun goal, so each survivor costs one trace of ~112 rays). A goal is Skyline-tier when
its cost function uses a `sun_*` feature.

**An entry standing in for a goal** (round 2: `sunset_behind`, `sunset_between_two`, `towers_over`,
`skyline_view`; round 1's `sunset_beside` etc. now the same way): its prune is measured with the
entry's own `must` on its own, its params bound to the scenario's towers. "The second tower" is the
next tallest building in the area, at least 60 m from the tower.

## Results (rounds 1 to 7: 18 places, 3 days)

<!-- library-lab:begin (generated by `--write`, don't edit by hand) -->

Places: pershing (LA, buildings from costar), bunker-hill (LA, buildings from dsm), south-park (LA, buildings from costar), little-tokyo (LA, buildings from costar), nassau-hall (Princeton, buildings from dsm), witherspoon (Princeton, buildings from dsm), dinky (Princeton, buildings from dsm), macarthur-park (LA, buildings from costar), grad-college (Princeton, buildings from dsm), civic-center (LA, buildings from dsm), fashion-district (LA, buildings from costar), chinatown (LA, buildings from costar), westlake (LA, buildings from costar), echo-park (LA, buildings from costar), convention-center (LA, buildings from costar), princeton-stadium (Princeton, buildings from dsm), mercer-west (Princeton, buildings from dsm), carnegie-north (Princeton, buildings from dsm). Days: 2026-11-14, 2026-06-21, 2026-12-21. Area radius 300 m, recall over the top 10.

Scenarios (shot classes), ungated. Runs with at least one spot, then medians over the runs: candidates searched, skyline traces, spots picked, time.

| scenario | standing | event | what it asks | runs with a spot | candidates | skyline traces | spots | time |
|---|---|---|---|---|---|---|---|---|
| `clear` | Any | sunset | sun in view down to the horizon, roof or street | 54 of 54 (18 places) | 550 | 550 | 10 | 1410 ms |
| `between` | Any | sunset | sun sets into a gap between buildings, roof or street | 51 of 54 (18 places) | 550 | 550 | 6 | 1383 ms |
| `behind` | Any | sunset | sun drops behind a building, roof or street | 54 of 54 (18 places) | 550 | 550 | 10 | 1512 ms |
| `street-between` | Ground | sunset | street level, sun sets into a gap | 21 of 54 (11 places) | 235 | 235 | 0 | 594 ms |
| `street-behind` | Ground | sunset | street level, sun drops behind a building | 53 of 54 (18 places) | 235 | 235 | 10 | 622 ms |
| `tower-and-sunset` | Any | sunset | sunset in view, and the tower must fill part of the frame (fills is a must) | 54 of 54 (18 places) | 550 | 144 | 10 | 516 ms |
| `soft-tower` | Any | sunset | as tower-and-sunset, but fills only weighted 0.5 (the agent's default shape) | 54 of 54 (18 places) | 550 | 550 | 10 | 1288 ms |
| `sun-by-tower` | Any | sunset | the sun must set within 15 deg of the tower; clear sky weighted 0.5 | 45 of 54 (17 places) | 550 | 550 | 5 | 1224 ms |
| `tower-framing-sunset` | Any | sunset | the tower must fill part of the frame and the sun must set within 15 deg of it | 25 of 54 (11 places) | 550 | 144 | 0 | 462 ms |
| `sunrise-by-tower` | Any | sunrise | sunrise: the sun must come up within 15 deg of the tower; clear sky weighted 0.5 | 50 of 54 (18 places) | 550 | 550 | 7 | 1178 ms |
| `silhouette` | Any | sunset | the sun drops behind the tower: sun_behind, with the sun within 5 deg of the tower (a must) | 32 of 54 (13 places) | 550 | 550 | 1 | 1197 ms |
| `between-towers` | Any | sunset | the sun sets between the two tallest buildings, both in one frame, in front of you (a must); clear sky weighted 0.5 | 39 of 54 (14 places) | 550 | 550 | 3 | 1189 ms |
| `looking-up` | Ground | sunset | street level, low angle: the tower looms over you (25+ deg tall, best 35+) and in view (a must); sun in view weighted 0.5 | 51 of 54 (17 places) | 235 | 11 | 3 | 121 ms |
| `rooftop-skyline` | Roof | sunset | on a roof, the tower reads as skyline (2-20 deg tall, best 4-12, in view; a must) with the sun setting within 15 deg of it (a must) | 24 of 54 (9 places) | 274 | 50 | 0 | 266 ms |
| `open-view` | Any | sunset | an open view: the sun stays in view to within 1 deg of the horizon (a must), ranked on clear sky | 45 of 54 (17 places) | 550 | 550 | 5 | 1213 ms |
| `sunrise-silhouette` | Any | sunrise | dawn silhouette: the sun comes up from behind the tower (sun_behind, sun within 5 deg of the tower; a must) | 33 of 54 (14 places) | 550 | 550 | 1 | 1074 ms |
| `sunrise-between-towers` | Any | sunrise | the sun rises between the two tallest buildings, both in one frame, in front of you (a must); clear sky weighted 0.5 | 38 of 54 (15 places) | 550 | 550 | 5 | 1093 ms |
| `lit-tower` | Any | sunset | golden light on the tower: the setting sun behind you (within 30 deg of straight behind), the tower in view (a must); fills weighted 0.5 | 49 of 54 (18 places) | 550 | 550 | 4 | 1204 ms |
| `sunrise-lit-tower` | Any | sunrise | morning light on the tower: the rising sun behind you (within 30 deg of straight behind), the tower in view (a must); fills weighted 0.5 | 42 of 54 (16 places) | 550 | 550 | 6 | 1031 ms |
| `sunrise-clear` | Any | sunrise | sunrise in view from the horizon up, roof or street (sun only) | 54 of 54 (18 places) | 550 | 550 | 10 | 1020 ms |
| `sunrise-open-view` | Any | sunrise | the library's open_horizon at sunrise: in view from within 1 deg of the horizon (a must), ranked on clear sky (sun only) | 50 of 54 (18 places) | 550 | 550 | 8 | 1028 ms |
| `sun-by-tower2` | Any | sunset | as sun-by-tower, on the second tallest building | 49 of 54 (18 places) | 550 | 550 | 7 | 1160 ms |
| `silhouette-tower2` | Any | sunset | as silhouette, on the second tallest building | 35 of 54 (16 places) | 550 | 550 | 1 | 1214 ms |
| `lit-tower2` | Any | sunset | as lit-tower, on the second tallest building | 51 of 54 (18 places) | 550 | 550 | 4 | 1269 ms |
| `sunrise-between` | Any | sunrise | the sun rises out of a gap between buildings, roof or street (sun only) | 51 of 54 (18 places) | 550 | 550 | 10 | 1292 ms |
| `sunrise-behind` | Any | sunrise | the sun comes up from behind a building, roof or street (sun only) | 54 of 54 (18 places) | 550 | 550 | 10 | 1227 ms |
| `sunrise-wide-gap` | Any | sunrise | the library's sun_wide_gap at sunrise (a must; sun only) | 43 of 54 (17 places) | 550 | 550 | 7 | 1179 ms |
| `pair-and-sunset` | Any | sunset | sunset in view with the two tallest buildings both in the frame and in view (a must) | 44 of 54 (15 places) | 550 | 26 | 6 | 208 ms |
| `skyline-glow` | Any | sunset | the setting sun behind you lights both tallest buildings, both in one frame and in view (a must) | 19 of 54 (9 places) | 550 | 550 | 0 | 1229 ms |
| `sunrise-skyline-glow` | Any | sunrise | the rising sun behind you lights both tallest buildings, both in one frame and in view (a must) | 25 of 54 (10 places) | 550 | 550 | 0 | 1178 ms |
| `silhouette-w10` | Any | sunset | as silhouette, but the sun within 10 deg of the tower | 37 of 54 (14 places) | 550 | 550 | 2 | 1187 ms |
| `silhouette-w20` | Any | sunset | as silhouette, but the sun within 20 deg of the tower | 43 of 54 (15 places) | 550 | 550 | 3 | 1379 ms |
| `lit-tower-w20` | Any | sunset | as lit-tower, but the sun within 20 deg of straight behind you | 44 of 54 (18 places) | 550 | 550 | 2 | 1265 ms |
| `down-the-street` | Any | sunset | street level, the sun sets down a street: a 1.5-20 deg gap, in view to 2 deg (a must, street written as a goal; sun only) | 21 of 54 (11 places) | 550 | 550 | 0 | 1191 ms |
| `sunrise-down-the-street` | Any | sunrise | as down-the-street, at sunrise | 27 of 54 (11 places) | 550 | 550 | 0 | 1290 ms |
| `keyhole` | Any | sunset | the library's sun_keyhole: the sun sets through a 1.5-3 deg slot (a must; sun only) | 40 of 54 (16 places) | 550 | 550 | 1 | 1233 ms |
| `wide-gap` | Any | sunset | the library's sun_wide_gap: the sun sets into a wide opening (6-20 deg) and stays in view in it (a must; sun only) | 48 of 54 (18 places) | 550 | 550 | 3 | 1218 ms |

Class (a) = speed gate, judged by recall@10 and prune. Class (b) = shot class (changes the result on purpose), judged by prune and a sane result.
Skyline traces are summed over all runs of the row. Ray what-ifs: traces and time are estimates (the gate is evaluated in the lab, not in a search).

| gate | class | kind | scenario | runs | places | tested runs (places) | prune median (min-max) | recall@10 min | near-best kept | skyline traces ungated -> gated | rays/cand (one trace) | time ungated -> gated |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `tower_framable` | (a) | entry | tower-and-sunset | 54 | 18 | 54 (18) | 4% (0%-64%) | 100% | 578/578 | 7257 -> 7257 | - | 523 -> 524 ms |
| `tower_framable` | (a) | entry | soft-tower | 54 | 18 | 54 (18) | 4% (0%-64%) | 10% | 509/534 | 27624 -> 24366 | - | 1472 -> 1348 ms |
| `tower_framable` | (a) | entry | tower-framing-sunset | 54 | 18 | 25 (11) | 4% (0%-64%) | 100% | 74/74 | 7257 -> 7257 | - | 526 -> 494 ms |
| `tower_toward_sunset` | (a) | entry | sun-by-tower | 54 | 18 | 45 (17) | 76% (11%-100%) | 100% | 333/333 | 27624 -> 10764 | - | 1423 -> 580 ms |
| `tower_toward_sunset` | (a) | entry | tower-framing-sunset | 54 | 18 | 25 (11) | 76% (11%-100%) | 100% | 74/74 | 7257 -> 2688 | - | 526 -> 260 ms |
| `tower_toward_sunset` | (a) | entry | silhouette | 54 | 18 | 32 (13) | 76% (11%-100%) | 100% | 73/73 | 27624 -> 10764 | - | 1389 -> 577 ms |
| `tower_toward_sunset` | (a) | entry | rooftop-skyline | 54 | 18 | 24 (9) | 76% (11%-100%) | 100% | 100/100 | 3234 -> 1419 | - | 315 -> 190 ms |
| `tower_toward_sunset` | (a) | entry | silhouette-w10 | 54 | 18 | 37 (14) | 76% (11%-100%) | 100% | 121/121 | 27624 -> 10764 | - | 1374 -> 585 ms |
| `tower_toward_sunrise` | (a) | entry | sunrise-by-tower | 54 | 18 | 50 (18) | 51% (6%-96%) | 100% | 317/317 | 27624 -> 11772 | - | 1319 -> 594 ms |
| `tower_toward_sunrise` | (a) | entry | sunrise-silhouette | 54 | 18 | 33 (14) | 51% (6%-96%) | 100% | 56/56 | 27624 -> 11772 | - | 1268 -> 563 ms |
| `tower_framed_toward_sunset` | (a) | entry | tower-framing-sunset | 54 | 18 | 25 (11) | 77% (12%-100%) | 100% | 74/74 | 7257 -> 2688 | - | 526 -> 258 ms |
| `sunset_beside` | (a) | entry | sun-by-tower | 54 | 18 | 45 (17) | 76% (11%-100%) | 100% | 333/333 | 27624 -> 10764 | - | 1423 -> 596 ms |
| `sunset_beside` | (a) | entry | sun-by-tower2 | 54 | 18 | 49 (18) | 67% (6%-98%) | 100% | 401/401 | 27624 -> 11124 | - | 1361 -> 623 ms |
| `sunrise_beside` | (a) | entry | sunrise-by-tower | 54 | 18 | 50 (18) | 51% (6%-96%) | 100% | 317/317 | 27624 -> 11772 | - | 1319 -> 593 ms |
| `tower_fills` | (a) | entry | tower-and-sunset | 54 | 18 | 54 (18) | 4% (0%-64%) | 100% | 578/578 | 7257 -> 7257 | - | 523 -> 536 ms |
| `sunset_behind` | (a) | entry | silhouette | 54 | 18 | 32 (13) | 76% (11%-100%) | 100% | 73/73 | 27624 -> 10764 | - | 1389 -> 592 ms |
| `sunset_behind` | (a) | entry | silhouette-tower2 | 54 | 18 | 35 (16) | 67% (6%-98%) | 100% | 84/84 | 27624 -> 11124 | - | 1362 -> 637 ms |
| `sunset_between_two` | (a) | entry | between-towers | 51 | 17 | 39 (14) | 76% (7%-100%) | 100% | 295/295 | 26181 -> 8403 | - | 1460 -> 574 ms |
| `towers_over` | (a) | entry | looking-up | 54 | 18 | 51 (17) | 93% (0%-100%) | 100% | 444/444 | 1332 -> 1332 | - | 152 -> 151 ms |
| `skyline_view` | (a) | entry | rooftop-skyline | 54 | 18 | 24 (9) | 14% (3%-100%) | 100% | 100/100 | 3234 -> 3234 | - | 315 -> 308 ms |
| `sunrise_behind` | (a) | entry | sunrise-silhouette | 54 | 18 | 33 (14) | 51% (6%-96%) | 100% | 56/56 | 27624 -> 11772 | - | 1268 -> 561 ms |
| `sunrise_between_two` | (a) | entry | sunrise-between-towers | 51 | 17 | 38 (15) | 63% (7%-100%) | 100% | 449/449 | 26181 -> 10950 | - | 1257 -> 617 ms |
| `tower_lit_at_sunset` | (a) | entry | lit-tower | 54 | 18 | 49 (18) | 36% (1%-94%) | 100% | 157/157 | 27624 -> 14895 | - | 1377 -> 811 ms |
| `tower_lit_at_sunset` | (a) | entry | lit-tower2 | 54 | 18 | 51 (18) | 43% (5%-97%) | 100% | 135/135 | 27624 -> 14358 | - | 1385 -> 775 ms |
| `tower_lit_at_sunrise` | (a) | entry | sunrise-lit-tower | 54 | 18 | 42 (16) | 61% (6%-99%) | 100% | 218/218 | 27624 -> 13305 | - | 1246 -> 686 ms |
| `both_in_frame` | (a) | entry | pair-and-sunset | 51 | 17 | 41 (14) | 18% (1%-92%) | 100% | 288/288 | 1950 -> 1950 | - | 245 -> 242 ms |
| `sun_down_the_street` | (a) | entry | down-the-street | 54 | 18 | 21 (11) | 50% (28%-65%) | 100% | 276/276 | 27624 -> 13641 | - | 1322 -> 645 ms |
| `sun_down_the_street` | (a) | entry | sunrise-down-the-street | 54 | 18 | 27 (11) | 50% (28%-65%) | 100% | 338/338 | 27624 -> 13641 | - | 1244 -> 607 ms |
| `skyline_glow` | (a) | entry | skyline-glow | 51 | 17 | 16 (8) | 57% (18%-100%) | 100% | 25/25 | 26181 -> 10044 | - | 1354 -> 614 ms |
| `skyline_glow_at_sunrise` | (a) | entry | sunrise-skyline-glow | 51 | 17 | 22 (9) | 80% (17%-100%) | 100% | 47/47 | 26181 -> 7773 | - | 1214 -> 492 ms |
| `tower_over_2_deg` | (a) | sweep | tower-and-sunset | 54 | 18 | 54 (18) | 0% (0%-0%) | 100% | 578/578 | 7257 -> 7257 | - | 523 -> 557 ms |
| `tower_under_40_deg` | (a) | sweep | tower-and-sunset | 54 | 18 | 54 (18) | 4% (0%-64%) | 100% | 578/578 | 7257 -> 7257 | - | 523 -> 543 ms |
| `tower_in_view` | (a) | sweep | tower-and-sunset | 54 | 18 | 54 (18) | 71% (29%-99%) | 100% | 578/578 | 7257 -> 7257 | - | 523 -> 547 ms |
| `on_roof` | (b) | sweep | clear | 54 | 18 | 54 (18) | 50% (35%-72%) | 70% | 1725/1807 | 27624 -> 13983 | - | 1528 -> 928 ms |
| `on_roof` | (b) | sweep | between | 54 | 18 | 51 (18) | 50% (35%-72%) | 30% | 175/215 | 27624 -> 13983 | - | 1501 -> 909 ms |
| `on_roof` | (b) | sweep | behind | 54 | 18 | 54 (18) | 50% (35%-72%) | 20% | 3784/6245 | 27624 -> 13983 | - | 1567 -> 1003 ms |
| `on_roof` | (b) | sweep | tower-and-sunset | 54 | 18 | 54 (18) | 50% (35%-72%) | 60% | 577/578 | 7257 -> 4617 | - | 523 -> 417 ms |
| `on_roof` | (b) | sweep | soft-tower | 54 | 18 | 54 (18) | 50% (35%-72%) | 70% | 532/534 | 27624 -> 13983 | - | 1472 -> 894 ms |
| `on_roof` | (b) | sweep | sun-by-tower | 54 | 18 | 45 (17) | 50% (35%-72%) | 0% | 179/333 | 27624 -> 13983 | - | 1423 -> 868 ms |
| `looms_3` | (a) | sweep | between | 54 | 18 | 51 (18) | 0% (0%-2%) | 100% | 215/215 | 27624 -> 27594 | - | 1501 -> 1506 ms |
| `looms_3` | (a) | sweep | behind | 54 | 18 | 54 (18) | 0% (0%-2%) | 100% | 6244/6245 | 27624 -> 27594 | - | 1567 -> 1642 ms |
| `looms_3` | (a) | sweep | street-between | 54 | 18 | 21 (11) | 0% (0%-4%) | 100% | 97/97 | 13641 -> 13614 | - | 697 -> 707 ms |
| `looms_3` | (a) | sweep | street-behind | 54 | 18 | 53 (18) | 0% (0%-4%) | 100% | 2510/2510 | 13641 -> 13614 | - | 712 -> 712 ms |
| `looms_west_3` | (a) | sweep | between | 54 | 18 | 51 (18) | 0% (0%-60%) | 33% | 212/215 | 27624 -> 26100 | - | 1501 -> 1401 ms |
| `looms_west_3` | (a) | sweep | behind | 54 | 18 | 54 (18) | 0% (0%-60%) | 0% | 6005/6245 | 27624 -> 26100 | - | 1567 -> 1471 ms |
| `looms_west_3` | (a) | sweep | street-between | 54 | 18 | 21 (11) | 0% (0%-52%) | 0% | 92/97 | 13641 -> 13029 | - | 697 -> 654 ms |
| `looms_west_3` | (a) | sweep | street-behind | 54 | 18 | 53 (18) | 0% (0%-52%) | 30% | 2424/2510 | 13641 -> 13029 | - | 712 -> 671 ms |
| `looms_8` | (a) | sweep | between | 54 | 18 | 51 (18) | 0% (0%-65%) | 10% | 203/215 | 27624 -> 25593 | - | 1501 -> 1414 ms |
| `looms_8` | (a) | sweep | behind | 54 | 18 | 54 (18) | 0% (0%-65%) | 0% | 5516/6245 | 27624 -> 25593 | - | 1567 -> 1381 ms |
| `looms_8` | (a) | sweep | street-between | 54 | 18 | 21 (11) | 0% (0%-62%) | 20% | 94/97 | 13641 -> 12600 | - | 697 -> 615 ms |
| `looms_8` | (a) | sweep | street-behind | 54 | 18 | 53 (18) | 0% (0%-62%) | 40% | 2248/2510 | 13641 -> 12600 | - | 712 -> 625 ms |
| `looms_west_8` | (a) | sweep | between | 54 | 18 | 51 (18) | 0% (0%-77%) | 10% | 165/215 | 27624 -> 21432 | - | 1501 -> 1070 ms |
| `looms_west_8` | (a) | sweep | behind | 54 | 18 | 54 (18) | 0% (0%-77%) | 0% | 4293/6245 | 27624 -> 21432 | - | 1567 -> 1082 ms |
| `looms_west_8` | (a) | sweep | street-between | 54 | 18 | 21 (11) | 0% (0%-76%) | 0% | 61/97 | 13641 -> 10545 | - | 697 -> 486 ms |
| `looms_west_8` | (a) | sweep | street-behind | 54 | 18 | 53 (18) | 0% (0%-76%) | 0% | 1553/2510 | 13641 -> 10545 | - | 712 -> 499 ms |
| `sunward_looms_3` | (a) | what-if | between | 54 | 18 | 51 (18) | 0% (0%-87%) | 10% | 195/215 | 27624 -> 23335 | - | 1501 -> 1225 ms |
| `sunward_looms_3` | (a) | what-if | behind | 54 | 18 | 54 (18) | 0% (0%-87%) | 0% | 5101/6245 | 27624 -> 23335 | - | 1567 -> 1238 ms |
| `sunward_looms_3` | (a) | what-if | street-between | 54 | 18 | 21 (11) | 0% (0%-83%) | 0% | 68/97 | 13641 -> 11743 | - | 697 -> 578 ms |
| `sunward_looms_3` | (a) | what-if | street-behind | 54 | 18 | 53 (18) | 0% (0%-83%) | 0% | 2063/2510 | 13641 -> 11743 | - | 712 -> 582 ms |
| `tower_on_sun_track` | (a) | what-if | sun-by-tower | 54 | 18 | 45 (17) | 94% (55%-100%) | 100% | 333/333 | 27624 -> 3507 | - | 1423 -> 275 ms |
| `tower_on_sun_track` | (a) | what-if | tower-framing-sunset | 54 | 18 | 25 (11) | 94% (55%-100%) | 100% | 74/74 | 7257 -> 877 | - | 526 -> 155 ms |
| `tower_on_sun_track` | (a) | what-if | sunrise-by-tower | 54 | 18 | 50 (18) | 93% (53%-100%) | 100% | 317/317 | 27624 -> 3766 | - | 1319 -> 246 ms |
| `tower_on_sun_track` | (a) | what-if | silhouette | 54 | 18 | 32 (13) | 94% (55%-100%) | 100% | 73/73 | 27624 -> 3507 | - | 1389 -> 269 ms |
| `tower_on_sun_track` | (a) | what-if | rooftop-skyline | 54 | 18 | 24 (9) | 93% (53%-100%) | 100% | 100/100 | 3234 -> 496 | - | 315 -> 138 ms |
| `tower_on_sun_track` | (a) | what-if | sunrise-silhouette | 54 | 18 | 33 (14) | 93% (53%-100%) | 100% | 56/56 | 27624 -> 3766 | - | 1268 -> 237 ms |
| `tower_opposite_sun_track` | (a) | what-if | lit-tower | 54 | 18 | 49 (18) | 83% (21%-100%) | 100% | 157/157 | 27624 -> 7140 | - | 1377 -> 441 ms |
| `tower_opposite_sun_track` | (a) | what-if | sunrise-lit-tower | 54 | 18 | 42 (16) | 85% (36%-100%) | 100% | 218/218 | 27624 -> 5757 | - | 1246 -> 370 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | clear | 54 | 18 | 54 (18) | 79% (38%-97%) | 100% | - | 27624 -> 7520 | 12.1 (116) | 1528 -> 554 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | between | 54 | 18 | 51 (18) | 79% (38%-97%) | 100% | - | 27624 -> 7520 | 12.1 (116) | 1501 -> 559 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | street-between | 54 | 18 | 21 (11) | 92% (47%-100%) | 100% | - | 13641 -> 1886 | 12.9 (116) | 697 -> 160 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | open-view | 54 | 18 | 45 (17) | 79% (38%-97%) | 100% | - | 27624 -> 7520 | 12.1 (116) | 1405 -> 532 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | sunrise-clear | 54 | 18 | 54 (18) | 77% (39%-97%) | 90% | - | 27624 -> 7821 | 11.7 (116) | 1265 -> 491 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | sunrise-open-view | 54 | 18 | 50 (18) | 77% (39%-97%) | 100% | - | 27624 -> 7821 | 11.7 (116) | 1259 -> 490 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | sunrise-between | 54 | 18 | 51 (18) | 77% (39%-97%) | 100% | - | 27624 -> 7821 | 11.7 (116) | 1275 -> 488 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | sunrise-wide-gap | 54 | 18 | 43 (17) | 77% (39%-97%) | 100% | - | 27624 -> 7821 | 11.7 (116) | 1243 -> 477 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | down-the-street | 54 | 18 | 21 (11) | 79% (38%-97%) | 100% | - | 27624 -> 7520 | 12.1 (116) | 1322 -> 496 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | sunrise-down-the-street | 54 | 18 | 27 (11) | 77% (39%-97%) | 100% | - | 27624 -> 7821 | 11.7 (116) | 1244 -> 481 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | keyhole | 54 | 18 | 40 (16) | 79% (38%-97%) | 100% | - | 27624 -> 7520 | 12.1 (116) | 1350 -> 501 ms |
| `ray_sun_seen_below_6` | (a) | ray what-if | wide-gap | 54 | 18 | 48 (18) | 79% (38%-97%) | 100% | - | 27624 -> 7520 | 12.1 (116) | 1330 -> 494 ms |
| `ray_sun_seen_below_1` | (a) | ray what-if | open-view | 54 | 18 | 45 (17) | 98% (66%-100%) | 100% | - | 27624 -> 1710 | 4.5 (116) | 1405 -> 151 ms |
| `ray_sun_seen_below_1` | (a) | ray what-if | sunrise-open-view | 54 | 18 | 50 (18) | 96% (85%-100%) | 100% | - | 27624 -> 1434 | 3.9 (116) | 1259 -> 111 ms |
| `ray_sun_seen_below_3` | (a) | ray what-if | clear | 54 | 18 | 54 (18) | 94% (54%-99%) | 30% | - | 27624 -> 3576 | 7.8 (116) | 1528 -> 299 ms |
| `ray_sun_seen_below_3` | (a) | ray what-if | between | 54 | 18 | 51 (18) | 94% (54%-99%) | 75% | - | 27624 -> 3576 | 7.8 (116) | 1501 -> 301 ms |
| `ray_sun_seen_below_3` | (a) | ray what-if | street-between | 54 | 18 | 21 (11) | 100% (64%-100%) | 100% | - | 13641 -> 457 | 8.1 (116) | 697 -> 64 ms |
| `ray_blocked_at_sunset` | (a) | ray what-if | behind | 54 | 18 | 54 (18) | 0% (0%-28%) | 100% | - | 27624 -> 26685 | 1.0 (116) | 1567 -> 1508 ms |
| `ray_blocked_at_sunset` | (a) | ray what-if | street-behind | 54 | 18 | 53 (18) | 0% (0%-4%) | 100% | - | 13641 -> 13599 | 1.0 (116) | 712 -> 714 ms |

Verdicts. A class (a) gate is kept when recall@10 is 100% in every run, on at least 2 places, with median prune at least 30%. Class (b) is reported, not judged on recall. The `soft-tower` rows (a gate on a goal that is only weighted) are left out of the verdicts.

- `tower_framable` [entry] (Tower fits the frame: a looks 2-40 deg tall, the range where fills scores above 0): **weak**: full recall, but median prune only 4%
- `tower_toward_sunset` [entry] (Tower toward the sunset: a lies at bearing 180-330 deg, which holds the sun from 20 deg up to sunset, all year, from the tropics to 45 deg N, plus 15 deg either side): **keep**: full recall (tested in 163 runs on 17 places), median prune 76%
- `tower_toward_sunrise` [entry] (Tower toward the sunrise: a lies at bearing 30-180 deg, the sunrise mirror of tower_toward_sunset): **keep**: full recall (tested in 83 runs on 18 places), median prune 51%
- `tower_framed_toward_sunset` [entry] (Tower framed toward the sunset: a looks 2-40 deg tall and lies at bearing 180-330 deg (tower_framable and tower_toward_sunset together)): **keep**: full recall (tested in 25 runs on 11 places), median prune 77%
- `sunset_beside` [entry] (Sun sets beside it: within 15 deg of a. Its must (a at bearing 180-330 deg) drops points before any skyline is traced, so use it as a must. For briefs like 'the sun sets beside the tower (lab: sun-by-tower)'): **keep**: full recall (tested in 94 runs on 18 places), median prune 68%
- `sunrise_beside` [entry] (Sun rises beside it: within 15 deg of a. Its must (a at bearing 30-180 deg) drops points before any skyline is traced, so use it as a must. For briefs like 'the sun rises beside the tower (lab: sunrise-by-tower)'): **keep**: full recall (tested in 50 runs on 18 places), median prune 51%
- `tower_fills` [entry] (Tower fills the frame: like the fills seed (8-25 deg tall best, and in view), plus a cheap must (2-40 deg tall) that drops points before any line-of-sight march): **weak**: full recall, but median prune only 4%
- `sunset_behind` [entry] (Sun sets behind a (silhouette): the sun drops behind something within 5 deg of a. Its must (a at bearing 180-330 deg) drops points before any skyline is traced, so use it as a must. For briefs like 'silhouette: the sun drops behind the tower (lab: silhouette)'): **keep**: full recall (tested in 67 runs on 17 places), median prune 68%
- `sunset_between_two` [entry] (Sun sets between a and b, both in one frame (in front of you, not behind). Its must (a and b in one frame, neither at bearing 5-145 deg, where the sun can't be) drops points before any skyline is traced, so use it as a must. For briefs like 'the sun sets between two towers (lab: between-towers)'): **keep**: full recall (tested in 39 runs on 14 places), median prune 76%
- `towers_over` [entry] (a towers over you (a low-angle shot looking up): at least 25 deg tall, best from 35, and in view. Its must (25 deg tall) drops points before any line-of-sight march. For briefs like 'low angle, looking up at the tower (lab: looking-up)'): **keep**: full recall (tested in 51 runs on 17 places), median prune 93%
- `skyline_view` [entry] (a reads as part of a skyline, e.g. from a roof across town: 2-20 deg tall, best 4-12, and in view. Its must (2-20 deg tall) drops points before any line-of-sight march): **weak**: full recall, but median prune only 14%
- `sunrise_behind` [entry] (Sun rises behind a (a dawn silhouette): the sun comes up from behind something within 5 deg of a. Its must (a at bearing 30-180 deg) drops points before any skyline is traced, so use it as a must. For briefs like 'dawn silhouette behind the tower (lab: sunrise-silhouette)'): **keep**: full recall (tested in 33 runs on 14 places), median prune 51%
- `sunrise_between_two` [entry] (Sun rises between a and b, both in one frame (in front of you, not behind). Its must (a and b in one frame, neither at bearing 215-355 deg, where the rising sun can't be) drops points before any skyline is traced, so use it as a must. For briefs like 'the sun rises between two towers (lab: sunrise-between-towers)'): **keep**: full recall (tested in 38 runs on 15 places), median prune 63%
- `tower_lit_at_sunset` [entry] (The setting sun lights a (golden light on the face you see): the sun is behind you, within 30 deg of straight behind, and a is in view. Its must (a at bearing 345-165 deg, the only side a sunset-lit face can be on) drops points before any skyline is traced, so use it as a must. For briefs like 'golden light on the tower (lab: lit-tower)'): **keep**: full recall (tested in 100 runs on 18 places), median prune 41%
- `tower_lit_at_sunrise` [entry] (The rising sun lights a (morning light on the face you see): the sun is behind you, within 30 deg of straight behind, and a is in view. Its must (a at bearing 195-15 deg, the only side a sunrise-lit face can be on) drops points before any skyline is traced, so use it as a must. For briefs like 'morning light on the tower (lab: sunrise-lit-tower)'): **keep**: full recall (tested in 42 runs on 16 places), median prune 61%
- `both_in_frame` [entry] (a and b both in one frame and both in view (e.g. two towers in the shot). Its must (a and b within a lens angle of each other) drops points before any line-of-sight march): **weak**: full recall, but median prune only 18%
- `sun_down_the_street` [entry] (Street level, the sun sets (or rises) down a street: into a 1.5-20 deg gap between buildings on both sides, and in view in it to within 2 deg of the horizon. Sun only, no objects needed. Its must (street level, not on a roof) drops points before any skyline is traced): **keep**: full recall (tested in 48 runs on 15 places), median prune 50%
- `skyline_glow` [entry] (The setting sun lights a and b together, both in one frame (the skyline glows): the sun behind you, within 30 deg of straight behind each, both in view. Its must (a and b in one frame, both at bearing 345-165 deg) drops points before any skyline is traced, so use it as a must): **keep**: full recall (tested in 16 runs on 8 places), median prune 57%
- `skyline_glow_at_sunrise` [entry] (The rising sun lights a and b together, both in one frame (the skyline glows): the sun behind you, within 30 deg of straight behind each, both in view. Its must (a and b in one frame, both at bearing 195-15 deg) drops points before any skyline is traced, so use it as a must): **keep**: full recall (tested in 22 runs on 9 places), median prune 80%
- `tower_over_2_deg` [sweep]: **weak**: full recall, but median prune only 0%
- `tower_under_40_deg` [sweep]: **weak**: full recall, but median prune only 4%
- `tower_in_view` [sweep]: **keep**: full recall (tested in 54 runs on 18 places), median prune 71%
- `on_roof` [sweep]: **shot class**: median prune 50%, a result in 267 of 270 runs (ungated top 10 kept: 0% at worst)
- `looms_3` [sweep]: **weak**: full recall, but median prune only 0%
- `looms_west_3` [sweep]: **drop**: loses top spots in echo-park 2026-11-14 between (80%); echo-park 2026-11-14 behind (80%); echo-park 2026-11-14 street-between (83%); echo-park 2026-11-14 street-behind (90%); echo-park 2026-06-21 behind (80%); echo-park 2026-06-21 street-between (80%); convention-center 2026-11-14 between (33%); convention-center 2026-11-14 behind (10%); convention-center 2026-11-14 street-between (0%); convention-center 2026-11-14 street-behind (50%); convention-center 2026-06-21 between (40%); convention-center 2026-06-21 behind (0%); convention-center 2026-06-21 street-between (20%); convention-center 2026-06-21 street-behind (30%); convention-center 2026-12-21 between (40%); convention-center 2026-12-21 behind (20%); convention-center 2026-12-21 street-between (50%); convention-center 2026-12-21 street-behind (60%)
- `looms_8` [sweep]: **drop**: loses top spots in little-tokyo 2026-06-21 behind (80%); little-tokyo 2026-06-21 street-behind (80%); little-tokyo 2026-12-21 between (90%); little-tokyo 2026-12-21 behind (90%); little-tokyo 2026-12-21 street-behind (90%); macarthur-park 2026-06-21 between (90%); macarthur-park 2026-06-21 street-between (60%); macarthur-park 2026-06-21 street-behind (90%); macarthur-park 2026-12-21 between (90%); macarthur-park 2026-12-21 street-between (90%); chinatown 2026-11-14 between (75%); chinatown 2026-11-14 behind (90%); chinatown 2026-11-14 street-behind (90%); chinatown 2026-06-21 street-behind (40%); chinatown 2026-12-21 behind (80%); chinatown 2026-12-21 street-behind (90%); echo-park 2026-11-14 between (30%); echo-park 2026-11-14 behind (10%); echo-park 2026-11-14 street-between (50%); echo-park 2026-11-14 street-behind (60%); echo-park 2026-06-21 between (10%); echo-park 2026-06-21 behind (30%); echo-park 2026-06-21 street-between (20%); echo-park 2026-06-21 street-behind (70%); echo-park 2026-12-21 between (20%); echo-park 2026-12-21 behind (0%); echo-park 2026-12-21 street-between (50%); echo-park 2026-12-21 street-behind (40%)
- `looms_west_8` [sweep]: **drop**: loses top spots in little-tokyo 2026-11-14 between (90%); little-tokyo 2026-11-14 behind (70%); little-tokyo 2026-11-14 street-between (0%); little-tokyo 2026-11-14 street-behind (30%); little-tokyo 2026-06-21 behind (40%); little-tokyo 2026-06-21 street-behind (30%); little-tokyo 2026-12-21 between (90%); little-tokyo 2026-12-21 behind (80%); little-tokyo 2026-12-21 street-behind (40%); nassau-hall 2026-11-14 behind (90%); nassau-hall 2026-06-21 street-behind (90%); nassau-hall 2026-12-21 behind (80%); macarthur-park 2026-11-14 between (80%); macarthur-park 2026-11-14 street-between (90%); macarthur-park 2026-11-14 street-behind (90%); macarthur-park 2026-06-21 between (80%); macarthur-park 2026-06-21 street-between (60%); macarthur-park 2026-06-21 street-behind (80%); macarthur-park 2026-12-21 between (80%); macarthur-park 2026-12-21 street-between (70%); macarthur-park 2026-12-21 street-behind (90%); chinatown 2026-11-14 between (25%); chinatown 2026-11-14 behind (10%); chinatown 2026-11-14 street-behind (10%); chinatown 2026-06-21 between (20%); chinatown 2026-06-21 behind (50%); chinatown 2026-06-21 street-behind (20%); chinatown 2026-12-21 between (25%); chinatown 2026-12-21 behind (10%); chinatown 2026-12-21 street-behind (20%); westlake 2026-11-14 between (20%); westlake 2026-11-14 behind (40%); westlake 2026-11-14 street-between (40%); westlake 2026-11-14 street-behind (20%); westlake 2026-06-21 between (40%); westlake 2026-06-21 behind (40%); westlake 2026-06-21 street-between (30%); westlake 2026-06-21 street-behind (30%); westlake 2026-12-21 between (10%); westlake 2026-12-21 behind (40%); westlake 2026-12-21 street-between (22%); westlake 2026-12-21 street-behind (20%); echo-park 2026-11-14 between (20%); echo-park 2026-11-14 behind (10%); echo-park 2026-11-14 street-between (33%); echo-park 2026-11-14 street-behind (40%); echo-park 2026-06-21 between (10%); echo-park 2026-06-21 behind (0%); echo-park 2026-06-21 street-between (20%); echo-park 2026-06-21 street-behind (50%); echo-park 2026-12-21 between (10%); echo-park 2026-12-21 behind (0%); echo-park 2026-12-21 street-between (50%); echo-park 2026-12-21 street-behind (10%); convention-center 2026-11-14 between (11%); convention-center 2026-11-14 behind (10%); convention-center 2026-11-14 street-between (0%); convention-center 2026-11-14 street-behind (0%); convention-center 2026-06-21 between (20%); convention-center 2026-06-21 behind (0%); convention-center 2026-06-21 street-between (0%); convention-center 2026-06-21 street-behind (10%); convention-center 2026-12-21 between (20%); convention-center 2026-12-21 behind (20%); convention-center 2026-12-21 street-between (0%); convention-center 2026-12-21 street-behind (0%)
- `sunward_looms_3` [what-if]: **drop**: loses top spots in pershing 2026-06-21 behind (90%); south-park 2026-11-14 street-behind (80%); south-park 2026-06-21 between (30%); south-park 2026-06-21 behind (50%); south-park 2026-06-21 street-between (0%); south-park 2026-06-21 street-behind (10%); nassau-hall 2026-06-21 behind (80%); witherspoon 2026-06-21 behind (90%); witherspoon 2026-06-21 street-behind (90%); dinky 2026-06-21 behind (90%); dinky 2026-06-21 street-behind (90%); macarthur-park 2026-11-14 between (60%); macarthur-park 2026-11-14 behind (90%); macarthur-park 2026-11-14 street-between (40%); macarthur-park 2026-11-14 street-behind (60%); macarthur-park 2026-12-21 between (50%); macarthur-park 2026-12-21 street-between (70%); macarthur-park 2026-12-21 street-behind (50%); fashion-district 2026-11-14 between (50%); fashion-district 2026-11-14 behind (60%); fashion-district 2026-11-14 street-between (50%); fashion-district 2026-11-14 street-behind (50%); fashion-district 2026-12-21 between (30%); fashion-district 2026-12-21 behind (70%); fashion-district 2026-12-21 street-between (0%); fashion-district 2026-12-21 street-behind (40%); chinatown 2026-06-21 behind (90%); westlake 2026-11-14 between (70%); westlake 2026-11-14 behind (20%); westlake 2026-11-14 street-between (70%); westlake 2026-11-14 street-behind (10%); westlake 2026-06-21 between (90%); westlake 2026-06-21 street-between (80%); westlake 2026-12-21 between (50%); westlake 2026-12-21 behind (10%); westlake 2026-12-21 street-between (78%); westlake 2026-12-21 street-behind (30%); echo-park 2026-11-14 between (70%); echo-park 2026-11-14 behind (50%); echo-park 2026-11-14 street-between (50%); echo-park 2026-11-14 street-behind (70%); echo-park 2026-06-21 between (30%); echo-park 2026-06-21 behind (0%); echo-park 2026-06-21 street-between (20%); echo-park 2026-06-21 street-behind (50%); echo-park 2026-12-21 between (90%); echo-park 2026-12-21 behind (50%); echo-park 2026-12-21 street-between (75%); echo-park 2026-12-21 street-behind (80%); convention-center 2026-11-14 between (11%); convention-center 2026-11-14 behind (10%); convention-center 2026-11-14 street-between (0%); convention-center 2026-11-14 street-behind (30%); convention-center 2026-06-21 between (10%); convention-center 2026-06-21 behind (0%); convention-center 2026-06-21 street-between (0%); convention-center 2026-06-21 street-behind (0%); convention-center 2026-12-21 between (40%); convention-center 2026-12-21 behind (20%); convention-center 2026-12-21 street-between (50%); convention-center 2026-12-21 street-behind (60%)
- `tower_on_sun_track` [what-if]: **keep**: full recall (tested in 209 runs on 18 places), median prune 93%
- `tower_opposite_sun_track` [what-if]: **keep**: full recall (tested in 91 runs on 18 places), median prune 84%
- `ray_sun_seen_below_6` [ray what-if]: **drop**: loses top spots in mercer-west 2026-06-21 sunrise-clear (90%)
- `ray_sun_seen_below_1` [ray what-if]: **keep**: full recall (tested in 95 runs on 18 places), median prune 97%
- `ray_sun_seen_below_3` [ray what-if]: **drop**: loses top spots in pershing 2026-11-14 clear (90%); pershing 2026-06-21 clear (86%); bunker-hill 2026-12-21 between (80%); little-tokyo 2026-06-21 clear (30%); witherspoon 2026-11-14 clear (40%); witherspoon 2026-06-21 clear (90%); witherspoon 2026-12-21 clear (60%); dinky 2026-06-21 clear (80%); grad-college 2026-11-14 clear (67%); grad-college 2026-06-21 clear (67%); grad-college 2026-12-21 clear (60%); civic-center 2026-11-14 clear (30%); civic-center 2026-06-21 clear (60%); civic-center 2026-06-21 between (75%); civic-center 2026-12-21 clear (40%); fashion-district 2026-06-21 clear (30%); chinatown 2026-11-14 clear (90%); mercer-west 2026-06-21 clear (80%); mercer-west 2026-12-21 clear (80%); carnegie-north 2026-11-14 clear (30%); carnegie-north 2026-06-21 clear (30%); carnegie-north 2026-12-21 clear (80%)
- `ray_blocked_at_sunset` [ray what-if]: **weak**: full recall, but median prune only 0%

<!-- library-lab:end -->

## What we learned (round 1)

(Round 1 ran on the first 7 places; its numbers below are from that run. The table above now has all 9,
which moves a few medians, e.g. sunrise from 64% to 50%.)

1. **A gate is only safe as a precheck of a must.** On the agent's default shape (every goal
   weighted, the `soft-tower` control), even a gate that is exactly fills' own range drops top
   spots: recall falls to 10% at Bunker Hill. Those spots win on `sun_clear` while fills is 0 there:
   the tower is under 2 deg, or over 40 deg because they're on or right beside it (Bunker Hill's
   311 m tower). So the library's speed gates belong
   *inside* shot-class entries (an entry's own `must`) or next to a goal the brief makes a must.

2. **Tower direction is the big, safe win today.** For any shot that puts the sun beside a known
   object, `tower_toward_sunset` / `sunset_beside` drop a median 81% of candidates (11-96% by
   place) before any skyline is traced, with 100% recall in all 14 runs. Sunrise: 64%. It's provably
   safe from the tropics to 45 deg N: the cone holds the sun's path from 20 deg up to sunset all
   year, plus the 15 deg the relation allows. How much it prunes depends on where the tower sits in
   the area. It cuts least when the tower is west of most of the area (Nassau Hall: 11%).

3. **Framing gates barely prune in a 300 m area round the tower.** `tower_framable` / `tower_fills`
   are safe but drop a median of only 4%. The tower is 2-40 deg tall from almost everywhere nearby,
   and fills (Ray tier) already runs before the skyline, so they save line-of-sight marches, not
   traces. They're kept as entries for the record and for briefs whose area isn't round the tower.

4. **Objects can't stand in for the skyline.** "Some building looks >= N deg tall" prunes 0% at
   3 deg: something tall is always near, downtown or among Princeton's trees, which the height map
   counts as buildings. At 8 deg, or aimed west, it starts losing top spots. Even aimed exactly at
   the day's sun track (`sunward_looms_3`) it loses them (recall down to 0%). Gaps and blockers come
   from geometry that isn't in the object list: CoStar is partial (no Bunker Hill towers), and a
   `kind` reference binds only the 64 objects nearest the centre. The skyline sees all of it; the
   objects don't.

5. **`on_roof` (b) isn't a free speed-up.** It drops ~49% of candidates, but street spots make the
   ungated top 10 often (Little Tokyo, Princeton, behind, sun-by-tower). As a shot class it's
   already `standing: roof`, so it isn't saved as an entry.

6. **The most valuable number: a ~12-ray precheck replaces most skyline traces.**
   `ray_sun_seen_below_6` asks "does the sun come into view before it is 6 deg up?" (that's
   `hidden_elev_deg < 6`, which `sun_between` needs and the best `sun_clear` spots have). It
   computes this exactly, with rays at the fast pass's own skyline azimuths, only those the track
   crosses below 6 deg. It drops a median 90% of candidates (75-100%) at 100% recall on clear,
   between and street-between in all 42 runs, for 12 rays per candidate against 112 for a trace.
   Traces fall from 6486 to 885 for clear, an estimated 4.5x faster search. The 3 deg version is
   safe for between but loses clear spots.
   `behind` has no cheap precheck yet: the sun is behind something at sunset almost everywhere
   (`ray_blocked_at_sunset` prunes 0%).

7. **A Point "sun track" feature would turn the 81% into 94%.** The what-if `tower_on_sun_track`
   (the tower within 15 deg of *this day's* sun azimuths, fixed per brief here) prunes a median 94%
   at 100% recall, against 81% for the year-round cone.

## What we learned (round 2)

Round 2 added five shot classes, four entries, two places (MacArthur Park in LA, Princeton's Graduate
College, both inside the cached height maps), a `--radius` flag and a diagnosis of the framing gates.
Same rules: offline, cache only, no core src changes.

1. **Four new shot classes, four new entries kept.** Each entry is the shot class with its speed gate
   as its own `must`, measured against the same brief with that must removed (recall 100% in all 18
   runs on 9 places for every one):
   - `silhouette` (the sun drops behind the tower) -> **`sunset_behind`**: prunes a median 81%
     (11-96%), skyline traces 8248 -> 3348, search 713 -> 363 ms. The day's sun track
     (`tower_on_sun_track`, what-if) would make it 94% and 186 ms.
   - `between-towers` (the sun sets between the two tallest buildings, both in one frame) ->
     **`sunset_between_two`**: 76% (7-100%), traces 8326 -> 2724, 689 -> 313 ms. Its must is
     Point tier (in one frame, neither tower to the east), while the plain brief's must is all Skyline
     tier, so nothing was dropped before the trace. That's exactly what proposed task 066 (hoist
     Point conjuncts) would do automatically.
   - `looking-up` (street level, low angle: the tower 25+ deg tall and in view) -> **`towers_over`**:
     91% (0% at Bunker Hill, where the 311 m tower is over 25 deg from everywhere within 300 m). But it
     saves **no** traces and no time (107 -> 108 ms): the brief's must is Ray tier (`visible`), so it
     already runs before the skyline (a median 13 traces). It only saves line-of-sight marches.
   - `rooftop-skyline` (on a roof, the tower 2-20 deg tall and in view, the sun within 15 deg of it):
     `tower_toward_sunset` cuts 80% and halves the traces (788 -> 386). Its own entry
     `skyline_view` (the size range) is **weak**: 22%, for the reason in point 3.
2. **`open-view` has an exact, very cheap precheck.** The brief "the sun stays in view to within 1 deg
   of the horizon" is `sun_hidden_elev < 1`, and `ray_sun_seen_below_1` (ray what-if, task 065's
   feature at 1 deg) is that same test before any skyline: 97% pruned (92-100%), **4.4 rays per
   candidate against 113 for a trace**, traces 8248 -> 218, an estimated 673 -> 41 ms. Recall is 100%
   by construction, and measured.
3. **Why `tower_framable` / `tower_fills` prune only 4%: fills' cut is its line of sight, not its
   size range.** Split into parts on `tower-and-sunset` (sweeps): "tower over 2 deg" drops **0%**
   everywhere, "under 40 deg" drops 4% (only the cells right next to the tower), "tower in view" drops
   **76%** (50-87%). A tower looks 2 deg tall out to about 28x its height (3.5 km for a 126 m tower),
   so in any area we search the low end never bites. A bigger area doesn't help: at `--radius 800`
   (8 places; MacArthur Park isn't cached that far) framable drops 3% and "in view" 87%. The tallest
   building in a bigger area is taller, and the heat grid keeps ~470 cells whatever the radius. And
   the line of sight (`visible`, Ray tier) already runs before the skyline: `tower-and-sunset` traces a
   median of 82 of 488 candidates. So no Point gate can save traces on fills-type shots, only cheap
   marches. Suggest leaving `tower_framable`, `tower_fills` and `skyline_view` out of SEEDS.
   (`tower_in_view` shows "keep" in the verdicts, but it's fills' own visibility part, a diagnostic,
   not an entry.)
4. **A gate found a bug in a brief.** The first version of `between-towers` tested "the sun is
   between them" as opposite signs of `sun_bearing_diff` to the two towers. That's also true with the
   sun right *behind* you (+179 and -179 deg). The entry's must ("neither tower to the east") lost
   recall (0-50% at Graduate College), and `LAB_DEBUG` showed the "spots" faced away from the sun.
   Fixed in both by adding |`sun_bearing_diff`| < 90. Worth knowing for the agent's own briefs:
   "between" written with signs needs a "facing it" term.
5. **Water isn't expressible yet.** Objects are buildings only (CoStar, or blocks found in the height
   map), and no feature sees a lake. MacArthur Park's lake looks like flat low ground. The closest
   today is `open-view`. A water shot would need water objects (e.g. from OSM) or a "flat, low
   surface toward the sun" ray feature. Not proposed as a task yet; say if you want one.
6. **Shot classes are sane.** Every new class finds spots in most runs (table at the top): looking-up
   18 of 18, open-view 16 of 18, between-towers 13 of 18, silhouette and rooftop-skyline 10 of 18.
   Both of the last two find nothing at Pershing, Bunker Hill or Witherspoon on either day. We haven't
   dug into why yet; a fair guess for rooftop-skyline is that the big downtown towers look over 20 deg
   tall from the roofs near them.

## What we learned (round 3)

Task 068. Round 3 added nine places, seven shot classes and six entries. The places are all inside the
cached height maps (nothing fetched): Civic Center, Fashion District, Chinatown, Westlake, Echo Park and
Convention Center in LA, and the stadium, Mercer West and north of Lake Carnegie in Princeton. Now that
there are 18 places, the older medians moved a little (e.g. sunset_beside went from 81% to 76%). **Read
recall with the "tested" count.** A run where the ungated search finds no spot counts as 100% recall
but proves nothing. So the promote list below gives the runs that had spots.

1. **Sunrise mirrors hold.** The sunset entries mirrored about north-south (bearing b -> 360 - b) keep
   100% recall at sunrise:
   - `sunrise_behind` (a dawn silhouette) prunes a median 51% in 21 tested runs on 13 places. Traces
     go from 18416 to 7848, and the search from 912 to 446 ms.
   - `sunrise_between_two` prunes 63% in 25 tested runs on 15 places, and the search goes from 929 to
     460 ms.
   Sunrise prunes less than sunset (51% against 76%) only because of where each place's tallest
   building happens to stand. The cone is just as wide.
2. **A new kind of shot: low sun on the tower.** "Golden light on the building" is a common
   filmmaker brief, and nothing in the library could express it before. The tower lights up when the
   sun is behind you, so the entry asks for |`sun_bearing_diff`| within 30 deg of 180 and the tower in
   view. The "absolute value" is written as max(x, -x) because the language has no `abs`.
   - Its must is the tower's bearing in the half opposite the low sun (345-165 deg at sunset,
     195-15 deg at sunrise). `bump` doesn't wrap around 360, so the must is an `or` of two bumps.
   - `tower_lit_at_sunset`: 100% recall in 32 tested runs on 18 places, but it prunes only 36%. The
     opposite cone has to be wide because the sun is "behind you" over a wide arc.
   - `tower_lit_at_sunrise`: 61%, in 28 tested runs on 16 places.
   - With the day's sun track (what-if `tower_opposite_sun_track`, task 064's feature turned around),
     they would prune 85% and 87%.
3. **Two sun-only entries that work today without objects.** `open_horizon` (the sun in view to
   within 1 deg of the horizon, ranked on clear sky) finds spots in 29 of 36 runs at sunset and 34 of
   36 at sunrise. `sun_wide_gap` (the sun sets into a 6-20 deg opening and stays in view in it) is a
   different shot from `sun_between` (narrow gaps, 1.5-6 deg) and finds spots in 32 of 36 runs on 17
   places. Neither has a cheap must today: their musts read the skyline. Task 065's `sun_seen_below`
   would make both cheap. At 1 deg it is exactly open_horizon's must: 98% pruned at sunset and 97% at
   sunrise, for about 4 rays a point. sun_wide_gap's score is 0 once the sun hides 6 deg up, so the
   6-deg test is provably safe for it: 80% pruned.
4. **The 6-deg ray precheck is only proven where the score needs it.** `ray_sun_seen_below_6` lost one
   spot at sunrise: Mercer West, June 21, `sunrise-clear`, recall 90%. `sun_clear` doesn't need the
   sun in view below 6 deg (70% of its score is golden-hour minutes in view). So for `sun_clear` the 6-deg test was
   only ever empirical, and the round 1/2 "100% on clear" was luck of the places. It stays safe, by
   construction, for `sun_between`, `sun_wide_gap` and open views. Task 065 should gate only goals
   whose score is 0 above the threshold. It shouldn't gate `sun_clear`.

## What we learned (round 4)

Round 4 added three things:
- **Dec 21**, the winter extreme of the sun track, as a third day. Jun 21 was already the summer one.
- **Every sunset entry re-run on the second tallest building** (`*-tower2`), so a result can't just
  come from where one particular tower stands.
- **The sun seeds at sunrise** (`sunrise-between`, `sunrise-behind`) and `sun_wide_gap` at sunrise.

The lab now runs places in parallel (about 12 minutes for everything), and the table has a
"tested runs" column. An entry is kept only if recall was actually tested on at least 2 places.

1. **The year-round cones hold at the winter solstice.** Every entry and what-if keeps 100% recall in
   every Dec 21 run (462 of 628 runs tested). The bearing cones claim to hold "all year". Jun 21 and
   Dec 21 are the two ends of the sun's yearly swing, so this is now measured, not only argued.
2. **The entries don't depend on which building.** On the second tallest building, recall stays 100%:
   - `sunset_beside`: 49 tested runs on 18 places, 67% pruned.
   - `sunset_behind`: 35 tested runs on 16 places, 67% pruned.
   - `tower_lit_at_sunset`: 51 tested runs on 18 places, 43% pruned.
   Medians move with where that building stands, as expected.
3. **The ray prechecks hold at sunrise too, where the score needs them.** `ray_sun_seen_below_6` keeps
   100% recall on `sunrise-between` (51 tested runs, 77% pruned) and `sunrise-wide-gap` (43, 77%).
   `ray_sun_seen_below_1` keeps 100% on both open views (95 tested runs, 97% pruned). The only loss is
   still `sun_clear` at sunrise (round 3, point 4).
4. **The sun seeds work at sunrise.** `sunrise-behind` finds spots in 54 of 54 runs, `sunrise-between`
   in 51 of 54, and `sunrise-wide-gap` in 43 of 54. So `sun_between`, `sun_behind` and `sun_wide_gap`
   are fine for sunrise briefs as they are.
5. **Silhouette is the pickiest shot class:** it finds spots in 32 of 54 runs on 13 places. On the
   second tower it finds spots in 35 of 54. The sun has to hide behind something within 5 deg of the
   named building, at 1.5-6 deg up, and in dense downtown a nearer building usually hides it first.
   That's a fair guess, not yet checked spot by spot. The entry is still exact: wherever there are
   spots, it keeps them all.

## What we learned (round 5)

Round 5 added shots with two buildings in one frame, and re-ran everything at a 500 m radius to check
the entries hold on bigger areas. The 500 m run printed only; the table above is for 300 m.

1. **Skyline glow, a new shot class: the low sun lights both towers in one frame.** "The skyline glows
   at sunset" is a classic city shot. It's rare at these places: spots in 19 of 54 runs at sunset and
   25 of 54 at sunrise. The two towers have to stand together on the side away from the sun.
   - `skyline_glow`: 100% recall in 16 tested runs on 8 places, 57% pruned, traces 26181 -> 10044.
   - `skyline_glow_at_sunrise`: 100% recall in 22 tested runs on 9 places, 80% pruned, traces
     26181 -> 7773.
   - Their must is the lit-tower cone on both buildings, plus "in one frame".
2. **`both_in_frame` (two towers in the shot) is weak: 18%.** It's like `towers_over`. The plain brief's
   must already runs on line of sight (Ray tier), before the skyline (a median of 26 traces). So the
   entry's cheap `in_frame` must only saves marches. It's kept in library/ as a building block, not for
   SEEDS.
3. **Everything holds at 500 m.** With a 500 m area every entry keeps 100% recall in every run (18
   places, 3 days). "The tower" is then the tallest building within 500 m, often a different building,
   which tests the entries again.
   - Prune moves with where that tower stands: `sunset_beside` 54% (68% at 300 m),
     `sunrise_beside` 82% (51%), `sunset_between_two` 81%, `skyline_glow` 89%.
   - `tower_lit_at_sunrise` drops to 24% there. It's still exact, only less of a speed-up.
   - The 6-deg ray precheck kept every spot at 500 m, including `sunrise-clear` (450 tested runs). The
     300 m loss at Mercer West still stands, so the advice in round 3 doesn't change.

## What we learned (round 6)

Shot-class tuning: how wide can "behind the tower" and "the sun behind you" be before the shot stops
being the shot? These are sweeps of the brief, not gates.

1. **Why silhouette is rare: distance.** `sun_behind` wants the sun to hide 1.5-6 deg up (and gives up
   by 15 deg). For the sun to hide behind *the tower* in that window, you have to stand roughly 10-40x
   the tower's height away: 1.2-5 km for Pershing's 126 m tower, 3-12 km for Bunker Hill's 311 m one. A
   300 m area round the tower is far too close, so the sun hides behind the tower too high up, or
   behind a nearer building first. Where the tower stands at the west edge of the area, almost no
   standpoint has it toward the sunset at all: the direction cone keeps 3 of 745 candidates at
   Convention Center, 13 of 297 at Witherspoon and 17 of 230 at Mercer West. Those three never find a
   silhouette, even at 20 deg. **For the agent:** to silhouette a named tower, search an area 1-5 km
   east of it, not round it.
2. **Widening "behind the tower" from 5 to 10 deg finds a spot in 5 more runs (32 -> 37 of 54). 20 deg
   finds one in 6 more (43).** At Pershing and Civic Center the sun drops behind a building right next
   to the tower. The `sunset_behind` must (the tower toward the sunset, +-15 deg) still holds at 10
   deg: 100% recall in 37 tested runs, 76% pruned. 20 deg is outside the cone's margin, so the must
   isn't valid there. Suggest leaving `sunset_behind` at 5 deg (a true silhouette of the tower) and
   letting the agent loosen it in a brief when 5 finds nothing.
3. **Lit tower: 30 deg is the right width.** At 20 deg the brief finds spots in 44 of 54 runs, against
   49 of 54 at 30 deg. It isn't any more "lit" in a way the features can tell, so `tower_lit_at_*` stay at 30.

## What we learned (round 7)

Two sun-only street shots. They need no objects, so they work in any area today.

1. **`sun_down_the_street`, the first sun-only entry that saves time today.** The shot: at street level,
   the sun sets or rises down a street, into a 1.5-20 deg gap between the buildings on both sides, and
   stays in view to within 2 deg of the horizon (the "-henge" shot). Its must is "not on a roof", which
   is Point tier. So it drops street-level-impossible points before any trace, with no 065 needed:
   - 100% recall in 48 tested runs on 15 places (sunset and sunrise, 3 days).
   - A median 50% pruned (28-65%), skyline traces 55248 -> 27282, search 1283 -> 626 ms.
   - Stacked with 065's 6-deg precheck (provably safe, since the score is 0 once the sun hides 6 deg
     up), the precheck alone would drop 78%.
   - It's rarer than it sounds: spots in 21 of 54 runs at sunset and 27 of 54 at sunrise (11 places),
     the same runs where `street-between` finds any.
2. **`sun_keyhole`: the sun through a 1.5-3 deg slot.** It's a tighter, more dramatic `sun_between`. It
   finds spots in 40 of 54 runs on 16 places, roof or street. It has no cheap must today. With 065 at
   6 deg (provably safe) it would prune 79%, at 100% recall in 40 tested runs.

## Promote list (for SEEDS)

**Status (round 5):** task 070 promoted the 11 entries of round 4's list to core's SEEDS (marked
"seed since 070" below). New candidates since: `skyline_glow` and `skyline_glow_at_sunrise` (round 5), `sun_down_the_street` and
`sun_keyhole` (round 7).

All 15 are in core's SEEDS: the first 11 since task 070, and `sun_down_the_street`, `sun_keyhole`,
`skyline_glow` and `skyline_glow_at_sunrise` since task 068 (each `about` names its brief). The chat picks `open_horizon` and `sun_wide_gap` itself when asked for those shots.

Ranked for the demo. **Update (round 4): Ray's 053 (object sources) has merged**, so the server now
passes objects to the search: CoStar buildings in LA, and Vexcel detections where it detects. Object
entries can bind today wherever the area has a building object. In the lab, Princeton's buildings come
from the height map. The server's come from Vexcel detection, so in Princeton an object entry works
only where detection finds the building. "Sun only" entries need no objects at all.
Every entry below has **100% recall@10 in every run**: 3 days (Nov 14, Jun 21, Dec 21), 18 places,
and for the sunset entries both the tallest and the second tallest building. The entries up to round 5 also
keep 100% at a 500 m radius (round 5). Round 7's were run at 300 m only. "Tested" counts the runs
whose ungated search found spots (the others can't lose any). Prune is the median share of candidates
its own must drops before any skyline trace (min-max over places). Traces and time are summed or
averaged over all its runs, with and without it.

| rank | entry | brief it serves (lab scenarios) | works today? | tested runs (places) | prune median (min-max) | skyline traces | search time |
|---|---|---|---|---|---|---|---|
| 1 | `open_horizon` (seed since 070) | "an open view, the sun right down to the horizon", sunset or sunrise (`open-view`, `sunrise-open-view`) | **yes**, sun only | 95 of 108 (18) | none today (its must reads the skyline); 97% (66-100%) with 065 at 1 deg | 55248 -> 3144 with 065 | 1186 -> ~117 ms with 065 |
| 2 | `sun_wide_gap` (seed since 070) | "the sun framed by the skyline, with room around it" (`wide-gap`, `sunrise-wide-gap`) | **yes**, sun only | 91 of 108 (18) | none today; 78% (38-97%) with 065 at 6 deg | 55248 -> 15341 with 065 | 1130 -> ~431 ms with 065 |
| 3 | `sun_down_the_street` | "the sun setting (or rising) right down a street, buildings on both sides" (`down-the-street`, `sunrise-down-the-street`) | **yes**, sun only, and it saves time today | 48 of 108 (15) | 50% (28-65%), a Point must (street level); 78% with 065 at 6 deg | 55248 -> 27282 | 1283 -> 626 ms |
| 4 | `sun_keyhole` | "the sun through a narrow slot between buildings" (`keyhole`) | **yes**, sun only | 40 of 54 (16) | none today; 79% (38-97%) with 065 at 6 deg | 27624 -> 7520 with 065 | 1350 -> ~501 ms with 065 |
| 5 | `sunset_behind` (seed since 070) | "silhouette: the sun drops behind the tower" (`silhouette`, `silhouette-tower2`) | yes, needs a building object | 67 of 108 (17) | 68% (6-100%) | 55248 -> 21888 | 1210 -> 523 ms |
| 6 | `sunset_beside` (seed since 070) | "the sun sets beside the tower" (`sun-by-tower`, `sun-by-tower2`) | yes, needs a building object | 94 of 108 (18) | 68% (6-100%) | 55248 -> 21888 | 1219 -> 549 ms |
| 7 | `sunset_between_two` (seed since 070) | "the sun sets between two towers" (`between-towers`) | yes, needs a building object | 39 of 51 (14) | 76% (7-100%) | 26181 -> 8403 | 1229 -> 491 ms |
| 8 | `tower_lit_at_sunset` (seed since 070) | "golden light on the tower" (`lit-tower`, `lit-tower2`) | yes, needs a building object | 100 of 108 (18) | 41% (1-97%) | 55248 -> 29253 | 1192 -> 682 ms |
| 9 | `sunrise_between_two` (seed since 070) | "the sun rises between two towers" (`sunrise-between-towers`) | yes, needs a building object | 38 of 51 (15) | 63% (7-100%) | 26181 -> 10950 | 1102 -> 537 ms |
| 10 | `tower_lit_at_sunrise` (seed since 070) | "morning light on the tower" (`sunrise-lit-tower`) | yes, needs a building object | 42 of 54 (16) | 61% (6-99%) | 27624 -> 13305 | 1096 -> 585 ms |
| 11 | `sunrise_beside` (seed since 070) | "the sun rises beside the tower" (`sunrise-by-tower`) | yes, needs a building object | 50 of 54 (18) | 51% (6-96%) | 27624 -> 11772 | 1147 -> 527 ms |
| 12 | `sunrise_behind` (seed since 070) | "dawn silhouette behind the tower" (`sunrise-silhouette`) | yes, needs a building object | 33 of 54 (14) | 51% (6-96%) | 27624 -> 11772 | 1111 -> 514 ms |
| 13 | `skyline_glow` | "the skyline glows: low sun on both towers" (`skyline-glow`) | yes, needs two building objects | 16 of 51 (8) | 57% (18-100%) | 26181 -> 10044 | 1176 -> 549 ms |
| 14 | `skyline_glow_at_sunrise` | "morning glow on both towers" (`sunrise-skyline-glow`) | yes, needs two building objects | 22 of 51 (9) | 80% (17-100%) | 26181 -> 7773 | 1099 -> 443 ms |
| 15 | `towers_over` (seed since 070) | "low angle, looking up at the tower" (`looking-up`) | yes, needs a building object | 51 of 54 (17) | 93% (0-100%) | 1332 -> 1332 (saves line-of-sight marches only) | 138 -> 141 ms |

(Rows 1 and 2: tested runs and prune are the 065 ray what-if on those scenarios. The `open-view`
scenario writes the same must inline.)

How it's ranked:
1. The sun-only entries come first, because they work in any area, objects or not. `sun_down_the_street`
   is the only one with a cheap must today.
2. Then the sunset object shots, by how common the brief is. "Beside" and "behind" are the hero
   shots.
3. Then the sunrise mirrors.
4. The skyline-glow pair is new and rare (few tested runs), so it comes after the single-tower shots.
5. `towers_over` is last. It's correct, but its brief already gates on line of sight, so it saves no
   time.

Not for SEEDS:
- `tower_toward_sunset`, `tower_toward_sunrise` and `tower_framed_toward_sunset` are the bare
  direction gates, and they're already the musts inside the entries above. Seed one only if the agent
  needs a standalone "the tower is toward the sunset" gate for its own briefs.
- `tower_framable`, `tower_fills` and `skyline_view` are weak (4-14%). See round 2, point 3.
- `both_in_frame` is weak (18%). See round 5, point 2.

What-ifs (not entries) show what 064 would add: with the day's sun track, the sunset and sunrise
entries would prune a median 93% (209 tested runs), and the lit-tower ones 84% (91 tested runs).

## Proposed next (as desk tasks)

- **Point feature: the day's sun track.** E.g. `sun_track_diff(obj)`: how far the object's bearing
  is from the day's sun azimuths (sunset back to 20 deg up). Constant per search, Point tier.
  Numbers: 94% vs 81% prune on sun-beside shots, 100% recall.
- **Ray feature: sun seen low.** E.g. `sun_seen_below(deg)`: the exact precheck above, about 12 rays.
  Numbers: 90% median prune on clear/between at 100% recall, ~4.5x fewer rays.
- **Funnel: hoist Point conjuncts.** When a must's expression is an `and`/`mul` with Point-tier
  parts (fills' size range, in_front's distance ratio), check those parts before the Ray/Skyline
  parts. This makes the speed gates automatic instead of hand-written entries like `tower_fills`.

Round 2 adds numbers to these: 065 (sun seen low) at 1 deg gives `open-view` 97% at 4.4 rays; 066
(hoist Point conjuncts) is what makes `between-towers` cheap without a hand-written entry.

## Ideas for later rounds

Measure 064/065/066 once they're built (after judging) and retire the what-ifs. Shot classes over
objects other than "the tower" (a named landmark by `kind`, once objects have more kinds). Sunrise
versions of silhouette and between-towers. The rest of the cache is LA downtown and central
Princeton only, so more places means fetching new areas: ask first.

# Library lab how-to: add a shot to the library, no Rust needed

Task 067. You write a **cost function** for one kind of shot a filmmaker asks for: a small JSON file in
`library/`. The **lab** runs it on 18 real places over 3 days and says **keep**, **weak** or **drop**.
The results are in `docs/library-lab.md`. This page covers everything you need to start.

## 1. Setup (once)

1. Install Rust (https://rustup.rs) and Git. Clone the repo, then branch: `git checkout -b library/<your-shot>`.
2. **Height maps.** Choose one:
   - **Copy the cache (recommended):** ask burvy for the `golden-cache` folder. You need `vexcel/dsm/L4`
     (357 MB, all 18 places) and `vexcel/dsm/L0` (2 small tiles). With the copy you fetch nothing, and your
     numbers match the doc's exactly.
   - **Fetch with your own key:** save the key in a file outside the repo, or in `env/vexcel.env` (gitignored).
     Never paste it into a command, a chat or a commit. Then fetch one place at a time with `--fetch` (see 3).
     Each place needs 5-11 tiles of about 13 MB (up to ~120 MB). It takes a minute or two, and nothing is fetched
     twice. Vexcel limits how often you can call it: the code waits and retries on its own (`429`). Don't fetch
     big areas: one place is a 300 m circle, plus 1.5 km around it for the skyline.
3. **CoStar kit** (you have it): it gives LA its real buildings. Without it, LA uses buildings found in the
   height map, and the LA numbers differ from the doc's.

Every new PowerShell window:
```
$env:VEXCEL_CACHE_DIR = "C:/path/to/golden-cache"
$env:COSTAR_KIT_DIR   = "C:/path/to/costar-kit"        # the folder with index.md and data/
$env:VEXCEL_ENV_FILE  = "C:/path/to/vexcel.env"        # only if you fetch
$env:LAB_THREADS      = "8"                            # places at once; about your CPU core count
```
To keep commands short: `$lab = "cargo run -p golden-core --release --example library_lab --"`, then
`iex "$lab --plan"`. The first build takes a few minutes.

## 2. What a cost function is

```json
{
  "name": "sunset_beside",
  "about": "Sun sets beside it: within 15 deg of a. ... For briefs like 'the sun sets beside the tower'",
  "params": ["a"],
  "must":  {"op": "gt", "x": {"op": "bump", "x": {"f": "bearing", "obj": "a"}, "at": 255, "width": 75}, "than": {"num": 0}},
  "score": {"op": "bump", "x": {"f": "sun_bearing_diff", "obj": "a"}, "at": 0, "width": 15}
}
```
- `name` is the file name (`library/sunset_beside.json`), in snake_case. `about` is one plain line that
  ends with "For briefs like '...'".
- `params` names the buildings the shot is about: `a` (in the lab, the tallest building in the area) and
  `b` (the next tallest). Leave it out for sun-only shots.
- `score` (0 to 1) is the shot. `must` is a quick test: wherever it's 0, the spot is dropped **before** the
  expensive skyline step. That's the whole speed win.

**The building blocks** (`docs/contracts.md`, "Goals and cost functions"):
`{"f": feature}`, `{"num": 5}`, `ramp` (0 at `lo`, 1 at `hi`), `bump` (1 at `at`, 0 at plus or minus `width`),
`and/or/min/max/mul/add/avg` over `of: [...]`, `not`, and `lt/gt` with `than` (gives 0 or 1).

**Features.** Each has a cost: *cheap* = Point, *medium* = Ray, *expensive* = Skyline. A must saves time only
if it uses cheap or medium features.

| feature | cost | meaning |
|---|---|---|
| `roof` | cheap | 1 on a roof, 0 at street level |
| `ground_m`, `eye_m` | cheap | height of the ground where you stand / of your eye, metres |
| `dist` (obj) | cheap | metres to the building |
| `bearing` (obj) | cheap | compass direction to the building (0 north, 90 east, 270 west) |
| `bearing_diff` (a, b) | cheap | how far a is right of b in the frame, deg (0 = lined up) |
| `size_deg` (obj) | cheap | how tall the building looks, deg |
| `in_frame` (a, b) | cheap | 1 if both fit in one frame |
| `lens_deg` | cheap | the brief's lens angle |
| `visible` (obj) | medium | how much of the building you can see, 0 to 1 |
| `sun_clear`, `sun_between`, `sun_behind` | expensive | the three basic sun shots, 0 to 1 |
| `sun_visible_min` | expensive | golden-hour minutes with the sun in view |
| `sun_hidden_elev` | expensive | how high the sun is when buildings hide it, deg (0 = never hidden) |
| `gap_width_deg` | expensive | width of the gap the sun sets into, deg |
| `sun_bearing_diff` (obj) | expensive | where the sun sets, minus the building's direction, deg |

## 3. Commands

| command | what it does |
|---|---|
| `--plan` | per place: tiles needed, missing, MB cached. Searches and fetches nothing |
| `--try NAME` | **your main loop**: tests `library/NAME.json` on every cached place and all 3 days (about 1 min) |
| `--try NAME --event sunrise` / `--standing ground` (or `roof`) | the same, for sunrise or one standing |
| `--place NAME` | one of the 18 places only (names in `--plan`) |
| `--at NAME,LAT,LON` | a place of your own (fetch it first with `--fetch`) |
| `--fetch` | with `--place` or `--at`: downloads that place's missing tiles |
| `--scenario KEY --gate NAME` | reruns one row of the doc |

Every file in `library/` is checked when the lab starts. A typo stops it with a message that says where.

## 4. The keep rule, and how to read the output

`--try` makes the brief "your score, without your must", then searches again with the must. It prints a
table row and a verdict:
```
| `sunset_beside` | (a) | entry | try-sunset_beside | 54 | 18 | 45 (17) | 76% (11%-100%) | 100% | 431/431 | 27624 -> 10764 | - | 1303 -> 522 ms |
- `sunset_beside` [entry] (...): **keep**: full recall (tested in 45 runs on 17 places), median prune 76%
```
- **recall@10** must be **100%**: with your must, the search still finds all of its top 10 spots. Anything
  less means your must throws away good spots: **drop**.
- **prune** is the share of spots your must drops before the skyline step. Keep needs a median of **30% or more**.
  Below that it's **weak**.
- **tested runs (places)**: runs where there was anything to find. You need at least 2 places.

**Worked examples:**
1. `sunset_beside` (above): the sun can only set beside a building that stands to the west. So the must
   says "a is at bearing 180-330". That's cheap, and it's true whenever the score is above 0. **keep**, 76%.
2. `sunset_between_two`: the must is "a and b fit in one frame, and neither is east, where the sun
   never sets". **keep**, 76%.
3. `sun_down_the_street`: its must is "not on a roof", but its score doesn't say "street". So rooftop
   spots with a good score get dropped: recall 0-40%, **drop** as a speed gate. The lesson: a must has to
   follow from the score. If the shot means "street level", put that in the score, or use `--standing ground`.

## 5. Your PR

1. Add `library/<name>.json`. Run `--try <name>` (add `--event sunrise` for sunrise shots) until it says
   **keep**.
2. In `docs/library-lab.md`, add a section `## GOGI's entries` above `## Promote list`. Give each entry its
   `--try` row and verdict line, and one sentence on the shot.
3. Add a line to `docs/gogi/DEVLOG.md`, run `cargo test --workspace`, commit, push, and open a PR to `main`.
   Boss reviews. Commit only your own files (no `cache/`, no keys).

## 6. First 15 minutes

1. Set the env vars (1), then run `--plan`. Every place should show `0 missing`, or "only fine ones".
2. `--place nassau-hall --scenario sun-by-tower --gate sunset_beside`. You should see 280 searched, 10
   spots, `prune 11%  recall@10 100%` on all 3 days. That's the same as the doc's raw rows.
3. `--try sunset_beside`: **keep**, 45 runs on 17 places, 76% (these numbers are with the CoStar kit;
   without it, 47 runs and 59%). Times vary with your machine.
4. Copy `library/sunset_beside.json` to `library/my_test.json`, change its `name`, and narrow the must's bearing
   window to `"width": 20`. `--try my_test` should now **drop**: 96% pruned but recall 0%, because it
   throws away the real spots. Then delete the file.
5. Write down three briefs you'd like to serve. Each one is a shot.

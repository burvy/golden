# Contracts

The source of truth for every shared data shape and endpoint. Each shape has a sample in `fixtures/`.
Change a shape only in its own small PR, and update its fixture in the same PR (AGENTS.md rule 2).

The Rust structs live in `crates/contracts` (pure: no network or files, so `core` can depend on it).
IO crates fill them: e.g. `vexcel::get_dsm` returns a `contracts::DsmGrid`, and `vexcel::grid::{read_files, write_files}` do the disk part.
A contract PR changes this file, the struct in `crates/contracts`, and the fixture together.

## Conventions
- Angles in degrees. Azimuth clockwise from true north.
- Heights in metres, as Vexcel's DSM gives them: roughly **above sea level** (downtown LA ground ≈ 80–90 m).
  The exact vertical datum isn't stated by Vexcel; treat heights as comparable to each other, not to GPS altitude.
- Times in ISO 8601 **with the local offset of the place** (e.g. `2026-06-21T20:07:41-07:00`), never bare.
  Every endpoint that returns times takes `tz` (IANA name, default `America/Los_Angeles`). Dates (`YYYY-MM-DD`)
  are the local calendar day at the place.
- Lat/lon WGS84 in every API. EPSG:3857 is only used inside `crates/vexcel`.
- Field names: `az_deg`, `elev_deg`, `lat`, `lon`, `*_m` for metres, `t` for a time. JSON is `snake_case`.
- Unknown fields in a request are an error (400), so typos don't silently do nothing.

## Phone budget (iPhone 13 Pro)
The web app must run smoothly on an iPhone 13 Pro over mobile data. Every endpoint is sized for that:
- JSON responses stay under ~200 KB. Bulk numbers (heights) go in a separate binary file, never in JSON.
- `/api/scene` is ~400 KB at `detail=phone` (the default), ~1.5 MB at `detail=desktop`.
- Images: `max_px` defaults to 1024, capped at 2048. Textures on the phone stay ≤ 4096 px.
- Raw Vexcel tiles (3–55 MB) never leave the server.

## Shapes

| Shape | Status | Owner | Rust (`crates/contracts`) | Fixture |
|---|---|---|---|---|
| `DsmGrid` | defined | vexcel | `dsm::DsmGrid` | `dsm_grid.json` + `dsm_grid.f32` |
| `SunPath` | defined | core | `sun::SunPath` | `sun_path.json` |
| `HorizonProfile` | defined | core | `horizon::HorizonProfile` | `horizon_profile.json` |
| `Brief` | defined | server | `search::Brief` | `brief.json` (old style), `brief_goals.json` (with goals) |
| `Goal`, `CostFn`, `Expr`, `ObjRef`, `Object`, `Rated` | defined | core | `goal::*` | `brief_goals.json`, `search_result.json` |
| `Spot`, `SearchResult` | defined | core | `search::{Spot, SearchResult}` | `search_result.json` |
| `GoalLayer`, `Stage`, `Investigation` | proposed (057) | core | `search::{GoalLayer, Stage, Investigation, Verdict}` | `search_layers.json` |
| `SceneMeta` | defined | server | `scene::SceneMeta` | `scene_meta.json` + `scene.bin` |
| `PhotoMeta` | defined | server | `photo::PhotoMeta` | `photo_meta.json` |
| `Building` | defined | costar | `building::{Building, BuildingList}` | `buildings.json` |
| `ApiError` | defined | server | `error::ApiError` | `error.json` |
| `AgentRequest`, `CallSheet` | defined | server | `agent::{AgentRequest, CallSheet}` | `agent_request.json`, `call_sheet.json` |
| `ChatRequest`, `ChatReply` | defined | server | `agent::{ChatRequest, ChatReply, ChatMessage, ChatAsk}` | `chat_request.json`, `chat_reply.json` |
| `MeshAt` | defined | server | none yet (built with `json!` in `crates/server/src/mesh.rs`; the fixture isn't parse-tested) | `mesh_at.json` |

All fixtures are in `fixtures/`. `cargo test -p contracts` checks every fixture parses into its struct.
Fixture numbers are realistic but made up (except the sun times, which are NOAA's for downtown LA).

## Endpoints

Server: `http://localhost:8787` (`crates/server`). All `GET` parameters are query parameters.

| Endpoint | Returns | What for |
|---|---|---|
| `GET /api/health` | `ok` (text) | Is the server up |
| `GET /api/sun` | `SunPath` | Sun times and path over a flat horizon |
| `GET /api/horizon` | `HorizonProfile` | Skyline from a point, and when buildings hide the sun |
| `POST /api/search` | `SearchResult` | Best spots in an area for a `Brief` |
| `GET /api/scene` | `SceneMeta` (+ binary) | Heights for the 3D view around a spot |
| `GET /api/photo` | `PhotoMeta` (+ JPEG) | Vexcel aerial photo of the view |
| `GET /api/buildings` | `BuildingList` | CoStar buildings near a point (who to ask for access) |
| `GET /api/buildings/{id}` | `Building` | One building |
| `POST /api/agent` | `CallSheet` | Free-text brief in, call sheet out (the agent) |
| `POST /api/chat` | `ChatReply` | The brief as a chat: the agent asks up to 3 questions, then searches and returns the call sheet |
| `GET /api/tiles/vexcel/{z}/{x}/{y}` | JPEG/PNG tile, or **204** | Vexcel aerial map tiles for MapLibre (XYZ, z0–21). 204 = no imagery for us here: show the fallback (Esri World Imagery). 400 for an impossible tile. `Cache-Control` 7 days. |
| `GET /api/mesh/at` | `MeshAt`, or **404** | Is there a Vexcel textured 3D mesh at this point? 404 = none here: use the height-map 3D view |
| `GET /api/mesh/near/{lat}/{lon}/{r}/tileset.json?...` | 3D Tiles tileset JSON | Vexcel's tileset through our proxy (key hidden), trimmed to `r` m (50–20000) around the point. Child URIs are relative, so they stay on the proxy |
| `GET /api/mesh/near/{lat}/{lon}/{r}/tile.glb?...` | glTF binary (`model/gltf-binary`, Draco + KTX2) | One mesh tile. `Cache-Control` 7 days |

**Errors** (every endpoint): JSON `ApiError` `{"error": "human-readable message"}` with status:

| Status | When |
|---|---|
| 400 | Bad or missing parameter, unknown field, out-of-range value |
| 404 | Unknown id (`/api/buildings/{id}`, `/api/scene/{id}.bin`, `/api/photo/{id}.jpg`) |
| 422 | Valid but impossible: no sunset that day (polar), or no height data there (only LA and Houston are covered) |
| 502 | Vexcel failed (after retries) |
| 503 | Needs Vexcel data that isn't cached and the server has no Vexcel key |

**Slow first calls:** `horizon`, `search` and `scene` need the height map. The first call for a new area downloads
it (seconds for 300 m, about a minute for 1.5 km); later calls for the same area use the cache. Pre-fetch the demo area.

### GET /api/sun
`?lat=34.0522&lon=-118.2437&date=2026-06-21&tz=America/Los_Angeles&step_min=10`

| Param | Default | Meaning |
|---|---|---|
| `lat`, `lon` | required | Place |
| `date` | required | Local day, `YYYY-MM-DD` |
| `tz` | `America/Los_Angeles` | IANA time zone for the output times |
| `step_min` | `10` | Minutes between `points` (1–60) |

Returns `SunPath`. Flat horizon only (no buildings): use `/api/horizon` for "when does the sun go behind that tower".
422 if the sun doesn't rise or set that day.

### GET /api/horizon
`?lat=34.0490&lon=-118.2510&above_m=1.7&radius_m=1500&step_deg=1&date=2026-06-21&tz=America/Los_Angeles`

| Param | Default | Meaning |
|---|---|---|
| `lat`, `lon` | required | Observer |
| `above_m` | `1.7` | Eye height above the surface at that point. On a roof the surface *is* the roof. |
| `radius_m` | `1500` | How far to look for obstructions (50–3000) |
| `step_deg` | `1` | Azimuth step (0.25–10) |
| `az_from`, `az_to` | `0`, `360` | Only this range of azimuths (clockwise, may wrap past 360) |
| `date`, `tz` | none | If given, also return `sun`: when the buildings hide the sun that evening |

Returns `HorizonProfile`.

### POST /api/search
Body: `Brief` (JSON). Returns `SearchResult`: the best `max_spots` spots, best first, plus a coarse score heatmap.
The agent turns free text into a `Brief`; the web app can also build one from form fields.

### GET /api/scene
`?lat=34.0490&lon=-118.2510&radius_m=1000&detail=phone`

| Param | Default | Meaning |
|---|---|---|
| `lat`, `lon` | required | Centre (usually a spot) |
| `radius_m` | `1000` | Outer edge of the scene (100–3000) |
| `detail` | `phone` | `phone`: rings of 256×256 cells (~400 KB). `desktop`: 512×512 (~1.5 MB) |

Returns `SceneMeta`. Then fetch `data_url` (`GET /api/scene/{id}.bin`, `application/octet-stream`, cacheable forever
since the id is content-based) and slice it into rings with `byte_offset`.

### GET /api/photo
`?lat=34.0490&lon=-118.2510&kind=oblique&look_az_deg=290&radius_m=150&max_px=1024`

| Param | Default | Meaning |
|---|---|---|
| `lat`, `lon` | required | What to photograph (usually the spot) |
| `kind` | `oblique` | `oblique`: angled aerial view, like looking out from the spot. `ortho`: straight down |
| `look_az_deg` | none | Oblique only: the direction you want to look (e.g. the sunset azimuth). The server picks the Vexcel camera closest to it |
| `radius_m` | `150` | Area around the point to include (25–500) |
| `max_px` | `1024` | Longest image side (256–2048) |

Returns `PhotoMeta`. Then fetch `image_url` (`GET /api/photo/{id}.jpg`, `image/jpeg`).
Must show `attribution` next to the image (Vexcel licence).

### GET /api/buildings
`?lat=34.0490&lon=-118.2510&radius_m=150&limit=20`

| Param | Default | Meaning |
|---|---|---|
| `lat`, `lon` | required | Centre |
| `radius_m` | `150` | Search radius (max 2000) |
| `limit` | `20` | Max buildings (1–100) |

Returns `BuildingList`, nearest first. Only CoStar's commercial properties (1,581 in LA): most houses and many
small buildings are not in it, so an empty list is normal. `GET /api/buildings/{id}` returns one `Building` (404 if unknown).

### GET /api/mesh/at
`?lat=34.04934&lon=-118.25076`

| Param | Default | Meaning |
|---|---|---|
| `lat`, `lon` | required | The spot (the eye of the 3D view) |

Returns `MeshAt`, or 404 where Vexcel has no 3D mesh (e.g. Princeton): the app then uses the height-map 3D view.
Load `tileset_url` (relative to the API base) with a 3D Tiles viewer; its tiles come from
`/api/mesh/near/{lat}/{lon}/{r}/tile.glb?...`. `/api/mesh/tileset.json` and `/api/mesh/tile.glb` without `near/...` also exist: same, untrimmed.

Everything is cached on the server's disk. The Vexcel key is only needed for what isn't cached yet, so a cached area
keeps working without a key. Errors: 400 bad lat/lon/r or unknown field; 404 no mesh here, or unknown tile;
502 Vexcel failed; 503 not cached and the server has no Vexcel key.

### DsmGrid

A north-up square of surface heights (roofs, trees, ground) around a centre point, in ground-metre cells.
Made by `vexcel::get_dsm(lat, lon, radius_m)`. Width = height = `ceil(2 * radius_m / cell_m)`.

On disk it is two files: a JSON meta and a raw little-endian `f32` file named by `data_file`.

```json
{
  "center_lat": 34.049,
  "center_lon": -118.251,
  "cell_m": 1.0,
  "width": 64,
  "height": 64,
  "nodata": -9999.0,
  "source": {
    "provider": "vexcel",
    "collection": "n34w118-us-ca-losangeles-2026",
    "captured_from": "Sun Apr 19 19:06:57 UTC 2026",
    "captured_to": "Wed Apr 29 16:44:04 UTC 2026",
    "native_res_m": 0.062
  },
  "data_file": "dsm_grid.f32"
}
```

| Field | Meaning |
|---|---|
| `center_lat`, `center_lon` | The point asked for. It sits at the centre of the grid (between cells when the size is even). |
| `cell_m` | Cell size in **ground metres** (default 1.0). |
| `width`, `height` | Cells per row, rows. |
| `nodata` | Value of a cell with no data (`-9999`). Always check for it. |
| `source.provider` | `"vexcel"`, `"usgs-3dep"` (LiDAR backup outside Vexcel's area) or `"synthetic"` (fixtures). |
| `source.collection`, `captured_from`, `captured_to` | Which Vexcel flight or USGS LiDAR project, and when (USGS: survey year only, `captured_to` null). `null` for synthetic. |
| `source.native_res_m` | Finest source pixel used, ground metres, before downsampling (~0.062 in LA). |
| data | `width * height` `f32` heights in metres. **Row-major, row 0 = north edge, column 0 = west edge.** |

**Finding the cell for a lat/lon** (R = 6378137):
```
east_m  = (lon - center_lon) * pi/180 * R * cos(center_lat)
north_m = (lat - center_lat) * pi/180 * R
col = floor(width/2  + east_m  / cell_m)
row = floor(height/2 - north_m / cell_m)
index = row * width + col
```
Over the 1–3 km we use, this flat approximation is off by centimetres.

**Each cell holds the highest point inside it**, not the average, so a roof edge anywhere in the cell
counts as blocking the sun.

### SunPath
The sun over one local day at a place, over a **flat** horizon (refraction included, buildings not).

```json
{
  "lat": 34.0522, "lon": -118.2437, "date": "2026-06-21", "tz": "America/Los_Angeles",
  "sunrise": { "t": "2026-06-21T05:42:03-07:00", "az_deg": 60.66 },
  "sunset":  { "t": "2026-06-21T20:07:41-07:00", "az_deg": 299.34 },
  "golden_hour": {
    "morning": { "from": "2026-06-21T05:23:49-07:00", "to": "2026-06-21T06:18:17-07:00" },
    "evening": { "from": "2026-06-21T19:31:28-07:00", "to": "2026-06-21T20:25:54-07:00" }
  },
  "points": [ { "t": "2026-06-21T19:30:00-07:00", "az_deg": 294.22, "elev_deg": 6.27 } ]
}
```

| Field | Meaning |
|---|---|
| `sunrise`, `sunset` | `{t, az_deg}` when the sun's top edge crosses a flat horizon. `null` if it doesn't happen that day. |
| `golden_hour.morning/evening` | `{from, to}` while the sun's elevation is between −4° and +6°. `null` if it doesn't happen. |
| `points` | `{t, az_deg, elev_deg}` every `step_min` minutes, from 30 min before sunrise to 30 min after sunset (whole day if the sun never sets). |

### HorizonProfile
The skyline seen from one eye point: for each azimuth, the highest angle anything rises to.

```json
{
  "observer": { "lat": 34.0490, "lon": -118.2510, "ground_m": 102.0, "eye_m": 103.7 },
  "radius_m": 1500, "step_deg": 1,
  "points": [ { "az_deg": 290.0, "elev_deg": 12.4, "dist_m": 38.0 } ],
  "sun": {
    "date": "2026-06-21",
    "flat_sunset": "2026-06-21T20:07:41-07:00",
    "hidden_at": "2026-06-21T19:12:05-07:00",
    "hidden_az_deg": 292.1, "hidden_elev_deg": 12.4,
    "golden_min_visible": 0
  }
}
```

| Field | Meaning |
|---|---|
| `observer.ground_m` | DSM surface height under the observer (a roof, if on a roof). |
| `observer.eye_m` | `ground_m + above_m`: the height the horizon is computed from. |
| `points` | One per `step_deg` from `az_from`. `elev_deg` is the highest obstruction angle within `radius_m` (negative if everything is below eye level, e.g. on a tall roof). `dist_m` is how far away that obstruction is. With no height data in that direction, `dist_m` is `null` and `elev_deg` is `0` (flat horizon assumed). |
| `sun` | Only when `date` was given. `flat_sunset`: as in `SunPath`. `hidden_at`: last moment the sun's centre is above the skyline that evening, or `null` if nothing blocks it before the flat horizon. `hidden_az_deg`, `hidden_elev_deg`: where it disappears. `golden_min_visible`: minutes of the evening golden hour with the sun in view. |

### Brief
What the filmmaker wants. `POST /api/search` body.

```json
{
  "text": "sun setting between two towers, from a rooftop, late June",
  "area": { "lat": 34.0490, "lon": -118.2510, "radius_m": 400 },
  "date": "2026-06-24",
  "tz": "America/Los_Angeles",
  "event": "sunset",
  "standing": "roof",
  "framing": "between",
  "max_spots": 10
}
```

| Field | Default | Meaning |
|---|---|---|
| `text` | `""` | The brief as typed. Only for the agent and logs; search uses the fields below. |
| `area` | required | Where to look: centre and `radius_m` (50–800; bigger is slower). Optional `name`: a display name for people ("Downtown LA, around Grand Tower"); the search ignores it, left out when unknown. |
| `date` | required | Local day of the shoot. |
| `tz` | `America/Los_Angeles` | |
| `event` | `"sunset"` | `"sunset"` or `"sunrise"`. |
| `standing` | `"any"` | `"ground"` (street level), `"roof"` (on a roof, so needs access), or `"any"`. |
| `framing` | `"clear"` | `"clear"`: sun visible down to the horizon, as long as possible. `"between"`: sun sets into a gap between two buildings. `"behind"`: sun sets behind a building (silhouette). |
| `max_spots` | `10` | 1–50. |
| `goals` | `[]` | What the shot should do, see [Goals](#goals-and-cost-functions). Empty means `[{"use": "sun_<framing>"}]`, i.e. exactly the sunset search from before goals. Left out of the JSON while empty. |
| `lens_deg` | `60` | Horizontal lens angle for "in one frame" checks (60 is about 30 mm on full frame). |

### SearchResult and Spot
`POST /api/search` response.

```json
{
  "brief": { "...": "the Brief, with defaults filled in" },
  "spots": [
    {
      "id": "s-34.04912--118.25087-roof",
      "rank": 1, "score": 0.91,
      "lat": 34.04912, "lon": -118.25087,
      "standing": "roof", "ground_m": 131.5, "eye_m": 133.2,
      "sun": {
        "date": "2026-06-24",
        "flat_sunset": "2026-06-24T20:08:16-07:00",
        "hidden_at": "2026-06-24T19:58:40-07:00",
        "hidden_az_deg": 296.8, "hidden_elev_deg": 2.1,
        "golden_min_visible": 39
      },
      "why": ["Sun drops into a 9 m gap between two towers at 19:58", "39 min of golden hour in view", "Roof 46 m up"],
      "building_id": "costar:1234567"
    }
  ],
  "heat": { "center_lat": 34.0490, "center_lon": -118.2510, "cell_m": 20, "width": 40, "height": 40, "values": [0.0, 0.12, null] },
  "searched": 1600,
  "took_ms": 2310
}
```

| Field | Meaning |
|---|---|
| `brief` | The request, with defaults filled in. |
| `spots` | Best first. `rank` starts at 1. `score` 0–1, only comparable within one search. |
| `spot.id` | Stable for the same place and standing; use it as a key in the web app. |
| `spot.standing` | `"ground"` or `"roof"`. |
| `spot.ground_m`, `eye_m` | Surface height at the spot, and eye height (surface + 1.7 m). |
| `spot.sun` | Same shape and meaning as `HorizonProfile.sun`. For `event: "sunrise"`, `hidden_at` is when the sun first *appears* over the skyline. |
| `spot.why` | Short plain-English reasons, best first, to show on the spot card. |
| `spot.building_id` | CoStar building the spot is on (roof spots), or `null`. Look it up with `/api/buildings/{id}`. |
| `spot.scores` | One `{goal, score, why}` per goal of the brief (the implicit sun goal if it had none), same order. `score` 0–1. Defaults to `[]` when reading older results. `spot.score` stays the overall rank score; `sun` and `why` stay as they were. |
| `heat` | Coarse score map for the heatmap layer. Same grid layout as `DsmGrid` (row 0 = north, row-major). `values` are 0–1, `null` where nothing was searched. At most 40×40 = 1,600 values. |
| `searched`, `took_ms` | How many candidate points were scored, and how long it took. |
| `spot.investigation` | What the AI made of the spot after looking at it (photos, 3D): `{verdict, note}`, `verdict` one of `confirmed`, `caution`, `moved_up`, `moved_down`, `ruled_out`; `note` a few plain sentences. Left out if it didn't look. |
| `layers` | One per goal, same order as `spot.scores`: each goal's score at every candidate. See [Goal layers and stages](#goal-layers-and-stages). Left out by older servers. |
| `stages` | The funnel, in order: `{label, n, goal}`. Left out by older servers. |
| `spacing_m` | Picked spots are at least this far apart (`max(40, 2 × heat.cell_m)` today). |
| `objects` | The objects the goals measured from (their `ObjRef`s, resolved): `Object`s, see [Goals](#goals-and-cost-functions), so the map can mark the tower and the fountain. Left out when the goals name none. |

### Goal layers and stages
Proposed for frontend v2 (task 057; core fills them in task 059). The page re-weights goals and switches
musts on and off **in the browser**, from these numbers, without a second scoring engine: the formulas
only ever run in core. Editing a formula, adding a goal or "check the new spots" is a new `POST /api/search`.
Example: `fixtures/search_layers.json` (the mock's downtown, 4 goals, 1,264 candidates, 52 KB).

There is one candidate per heat cell, so a layer is exact, not a summary.

```json
"layers": [
  { "goal": "close_to_tower", "weight": 0, "must": true, "heat": { "...": "HeatGrid of raw 0-1 scores" }, "gate": [17, 18, 56] },
  { "goal": "sun_between", "weight": 1, "must": false, "heat": { "...": "HeatGrid" } }
],
"stages": [
  { "label": "Points in the area", "n": 1264, "goal": null },
  { "label": "Somewhere to stand", "n": 1213, "goal": null },
  { "label": "Close enough to Grand Tower", "n": 1179, "goal": "close_to_tower" },
  { "label": "The fountain in front of the tower", "n": 770, "goal": "in_front" }
],
"spacing_m": 40
```

| Field | Meaning |
|---|---|
| `layer.goal` | `Goal::name()`. |
| `layer.weight`, `layer.must` | As searched (`weight` is `Goal::weight_or_default()`: 0 for a must with no weight). |
| `layer.heat` | Same grid as `heat`. The goal's score 0–1 at each candidate, **raw** (not normalised), `null` exactly where `heat` is `null`. Core scores every goal at every candidate that has somewhere to stand, even ones a gate drops, so switching a must off is exact. |
| `layer.gate` | Cells (indexes into `values`) the cost function's own `must` drops, whatever `layer.must` says. It's part of the formula, so the browser can't switch it off. Usually empty, then left out. |
| `stage.label`, `n` | Plain words, and how many candidates are left after the stage. Fixed stages first (`goal: null`): points in the area circle, then somewhere to stand (= `searched`). Then one stage per goal that gates, cheapest first, in the order core ran them. |
| `stage.goal` | The goal whose gate it is, or `null`. |

**The browser rule** (core does the same; `rebuild` in `crates/contracts/tests/fixtures.rs` is the reference):
1. A cell is dropped if it is in any layer's `gate`, or a layer with `must` scores ≤ 0 there.
2. Ranking goals: the layers with `weight` > 0. If there are none, every layer with weight 1/n.
3. `raw` = Σ weight × score over the ranking goals, 0 if dropped. `heat` = raw / max raw (0 if max is 0).
4. Spots: cells with raw > 0, best first (ties to the lower index), each at least `spacing_m` from those
   already picked (flat distance between cell centres), up to `max_spots`. Cell centres use the `DsmGrid` formula.

With the searched weights this gives back `heat` and the order of `spots` (to 1e-3 and 1e-5°). After an edit,
spots the server didn't send have no sun times or reasons yet: the page shows their per-goal scores from the
layers and offers "check the new ones" (`POST /api/search`).

### Goals and cost functions
Part of `Brief` (`goals`). The design and the reasons are in `docs/metrics.md` (v2). Rust: `contracts::goal`.

A **goal** uses a library cost function by name, or brings one inline:
```jsonc
{ "use": "sun_between", "weight": 1.0 }
{ "use": "in_front", "args": { "a": { "kind": "tree" }, "b": { "id": "costar:1234567" } }, "must": true }
{ "def": { "name": "...", "about": "...", "score": { "f": "sun_clear" } }, "weight": 0.5 }
```

| Field | Default | Meaning |
|---|---|---|
| `use` | | Name of a library cost function (`library/*.json`, or saved at runtime). Exactly one of `use` / `def`. |
| `args` | `{}` | Object references for the function's `params`, by name. Only with `use`. |
| `def` | | An inline `CostFn` (written for this brief, not saved). |
| `weight` | `1` (`0` if `must`) | Share of the ranking score: a spot's `score` is the weighted sum of its goal scores, normalised so the best candidate is 1. |
| `must` | `false` | A gate: a candidate scoring 0 on this goal is dropped. |

A **cost function** (`CostFn`), also the format of library entries:

| Field | Meaning |
|---|---|
| `name` | snake_case, unique in the library. Shown as `Spot.scores[].goal`. |
| `about` | One line for people and the agent. Starts each spot's `why` for this goal. |
| `params` | Names of object parameters (default `[]`), filled in by a goal's `args`. |
| `must` | Optional expression. Where it's 0 the candidate is dropped, whatever the goal's weight. |
| `score` | Expression, clamped to 0–1. Higher is better. |

An **expression** (`Expr`) is a JSON tree that always evaluates to a number:
```jsonc
{"f": "sun_visible_min"}                        // feature; object features take "obj", or "a" and "b"
{"num": 0.5}                                    // number
{"op": "ramp", "x": E, "lo": 20, "hi": 5}       // 0 at lo, 1 at hi, linear, clamped (lo > hi allowed, lo != hi)
{"op": "bump", "x": E, "at": 0, "width": 10}    // 1 at `at`, falling to 0 at +-width (width > 0)
{"op": "and" | "or" | "min" | "max" | "mul" | "add" | "avg", "of": [E, ...]}   // at least one
{"op": "not", "x": E}
{"op": "lt" | "gt", "x": E, "than": E}          // 0 or 1
```
Parsing rejects an unknown `op`, a missing or extra key, and non-finite numbers, with a message that says
what to fix. Which **features** exist (and their tier, Point < Ray < Skyline) is core's business: `core::expr`
checks names and parameters before a search runs. The feature list is in `docs/metrics.md`.

An **object reference** (`ObjRef`): `{"id": "costar:1234567"}`, or `{"kind": "tree", "near": <ObjRef>, "within_m": 300}`
meaning the best-scoring object of that kind (optionally within `within_m` of another). Inside an expression an
object argument is either a parameter name (`"a"`) or an inline `ObjRef`. Kinds are label strings; the checked ones
are in `goal::OBJECT_KINDS` (`building`, `tree`, `car`, `rooftop HVAC unit`, `swimming pool`, `water tower`).

An **object** (`Object`), as object sources hand them to core:
`{ "id": "costar:1234567", "kind": "building", "lat": 34.0501, "lon": -118.2533, "base_m": 88.0, "top_m": 310.0, "radius_m": 30.0, "source": "costar" }`.
Heights are on the same datum as `DsmGrid`. Optional `name` for people when the source has one ("Grand Tower"), left out otherwise.

`Rated` (`Spot.scores[]`): `{ "goal": "sun_between", "score": 0.91, "why": "Sun sets into a gap between two buildings: ..." }`.

### SceneMeta (+ binary)
Heights for the 3D view, in rings of decreasing detail, so the phone draws ~200k cells instead of millions.

```json
{
  "id": "9f2c1a7e",
  "center_lat": 34.0490, "center_lon": -118.2510,
  "base_m": 70.0,
  "encoding": "u16le_dm",
  "nodata": 65535,
  "rings": [
    { "cell_m": 1.0,  "width": 256, "height": 256, "byte_offset": 0 },
    { "cell_m": 4.0,  "width": 256, "height": 256, "byte_offset": 131072 },
    { "cell_m": 16.0, "width": 256, "height": 256, "byte_offset": 262144 }
  ],
  "bytes": 393216,
  "data_url": "/api/scene/9f2c1a7e.bin",
  "attribution": "Height data © Vexcel Imaging US, Inc."
}
```

| Field | Meaning |
|---|---|
| `rings` | Square grids, all centred on the centre, each `width × cell_m` metres across. Same layout as `DsmGrid` (row 0 = north, row-major). An outer ring also covers the inner ones: draw it with the inner ring's area cut out. |
| `encoding` | `u16le_dm`: each cell is a little-endian `u16`; height in metres = `base_m + value / 10` (decimetres, covers 6.5 km). |
| `nodata` | `u16` value for "no data". |
| `byte_offset` | Where the ring starts in the binary. Its length is `width * height * 2`. |
| `bytes` | Total size of the binary. |
| each cell | The **highest** point in it (like `DsmGrid`), so coarse rings never hide a tower. |

### PhotoMeta
```json
{
  "id": "b81e44d0",
  "kind": "oblique",
  "image_url": "/api/photo/b81e44d0.jpg",
  "width": 1024, "height": 683,
  "look_az_deg": 284.0,
  "captured": "2026-04-19",
  "attribution": "Imagery © Vexcel Imaging US, Inc."
}
```

| Field | Meaning |
|---|---|
| `kind` | `"oblique"` or `"ortho"`. |
| `look_az_deg` | Direction the camera looks (oblique). Obliques come in about 4 directions, so it can be up to ~45° from the request. `null` for ortho. |
| `captured` | Capture date, `YYYY-MM-DD`. |
| `attribution` | Must be shown next to the image. |

### Building (CoStar)
`GET /api/buildings` returns `BuildingList` `{"buildings": [Building, ...]}`; `GET /api/buildings/{id}` returns one `Building`.

```json
{
  "id": "costar:1234567",
  "name": "Example Tower",
  "address": "123 S Example St, Los Angeles, CA 90013",
  "lat": 34.0493, "lon": -118.2516, "dist_m": 61.0,
  "height_m": 45.7, "floors": 12, "roof_m": 131.5,
  "property_type": "Office",
  "contacts": [ { "role": "leasing", "name": "Jane Doe", "company": "Example Realty", "phone": "+1 213 555 0100", "email": "jane@example.com" } ],
  "matterport": [ { "id": "AbCdEfGhIjK", "url": "https://my.matterport.com/show/?m=AbCdEfGhIjK" } ]
}
```

| Field | Meaning |
|---|---|
| `id` | `costar:<CoStar property id>`. |
| `name`, `address`, `property_type` | `null` if missing. |
| `dist_m` | From the query point (`/api/buildings` only; `null` from `/api/buildings/{id}`). |
| `height_m` | CoStar's `buildingHeightInFeet` in metres, above ground. `null` if missing. |
| `floors` | `null` if missing. |
| `roof_m` | Roof height from the DSM, same datum as all other heights. `null` if not computed. |
| `contacts` | Who to call. `role` is `"manager"`, `"leasing"`, `"owner"` or `"other"`; other fields may be `null`. May be empty. |
| `matterport` | Matterport 3D tours of the inside. May be empty. |

### ApiError
`{"error": "lat must be in [-90, 90] and lon in [-180, 180]"}`, with a status code from the table under Endpoints.

### MeshAt
From `GET /api/mesh/at`.
```json
{
  "collection": "us-ca-losangeles-downey-2020-2021",
  "tileset_url": "/api/mesh/near/34.04934/-118.25076/3000/tileset.json?collection=us-ca-losangeles-downey-2020-2021&layer=3d-cities",
  "attribution": "3D mesh © Vexcel Imaging US, Inc. · © Microsoft"
}
```

| Field | Meaning |
|---|---|
| `collection` | The Vexcel 3D collection covering the point. |
| `tileset_url` | Relative to the API base. A 3D Tiles tileset (load with e.g. 3d-tiles-renderer), trimmed to 3000 m around the point. |
| `attribution` | Must be shown on the 3D view (Vexcel licence). |

### ChatRequest and ChatReply
`POST /api/chat` is the brief as a short conversation, in front of the same agent as `/api/agent`. It's **stateless**: the
client sends the whole conversation every turn. Each turn the agent (Claude, else the offline mock) either asks **one**
short question with quick-reply choices, or settles the `Brief`, runs the agent's search -> call sheet steps and returns both.
It asks at most 3 questions, never about something the text already answers, and the first choice is always its default.
A question comes back in ~2 s; the final turn takes as long as `/api/agent`.

Request (`ChatRequest`):
```json
{
  "messages": [
    { "role": "user", "text": "fountain in front of a tower at sunset" },
    { "role": "agent", "text": "Where should I look?" },
    { "role": "user", "text": "Downtown LA" }
  ],
  "area": null, "date": null, "tz": "America/Los_Angeles", "go": false
}
```

| Field | Default | Meaning |
|---|---|---|
| `messages` | required | Oldest first. The first and the last are the user's (`role`: `user` or `agent`). At most 24, each at most 1000 characters. |
| `area`, `date`, `tz` | `null`, `null`, LA | As in `AgentRequest`: a given area or date wins over the chat. |
| `go` | `false` | "Search now": skip any remaining questions and use the defaults. Then the last message may be the agent's. |

Response (`ChatReply`): exactly one of `ask` or (`brief` + `sheet`).
```json
{
  "understood": ["Sunset", "Downtown LA"],
  "ask": { "question": "Rooftop or street level?", "choices": ["Either is fine", "Rooftop", "Street level"] },
  "brief": null, "sheet": null, "model": "mock"
}
```
- `understood`: the brief so far in plain words. While asking, only what the user said; once searching, every field,
  with the agent's own defaults ending in ` (my pick)`,
  and goals the search can't score yet ending in ` (not checked yet)`.
- `ask.choices`: quick replies, the default first. May be empty (free text).
- `brief`: the brief the chat settled on, including its `goals` (see [Goals](#goals-and-cost-functions)): the sun goal
  for the framing (`{"use": "sun_between"}`) plus object goals as data, with kinds in the user's words
  (`{"use": "in_front", "args": {"a": {"kind": "fountain"}, "b": {"kind": "tower"}}}`). `sheet`: the `CallSheet` (its
  `brief` may be relaxed or widened from it); its `steps` start with a `chat` step.
- `note`: one plain sentence to show with the results, e.g. what the search couldn't check yet. Left out when empty.
- `model`: who ran this turn, model id or `"mock"`.

### AgentRequest and CallSheet
`POST /api/agent` turns a typed brief into a call sheet. The agent (Claude when `ANTHROPIC_API_KEY` is set, else an offline mock)
reads the text, builds a `Brief`, calls `search`, `horizon`, `buildings` and `photo` as tools, and writes the sheet.
It can take 10–30 s; show progress in the web app.

Request (`AgentRequest`):
```json
{
  "text": "sun setting between two towers, from a rooftop, late June",
  "area": { "lat": 34.0490, "lon": -118.2510, "radius_m": 400 },
  "date": null,
  "tz": "America/Los_Angeles"
}
```

| Field | Default | Meaning |
|---|---|---|
| `text` | required | The brief as typed. |
| `area` | `null` | Where to look. `null`: the agent picks from the text, else downtown LA. |
| `date` | `null` | Local day. `null`: the agent picks from the text ("late June" → a date), else the next day. |
| `tz` | `America/Los_Angeles` | |

Response (`CallSheet`):
```json
{
  "title": "Sun between two towers, DTLA rooftop",
  "brief": { "...": "the Brief the agent searched with" },
  "summary": "Shoot from the roof of Example Tower on Wed 24 June. The sun drops into a 9 m gap between two towers at 19:58.",
  "timeline": [
    { "t": "2026-06-24T19:00:00-07:00", "what": "Arrive and set up on the roof" },
    { "t": "2026-06-24T19:19:40-07:00", "what": "Golden hour starts" },
    { "t": "2026-06-24T19:58:40-07:00", "what": "Sun enters the gap: the shot" }
  ],
  "spots": [
    { "role": "primary", "spot": { "...": "Spot" }, "arrive_by": "2026-06-24T19:00:00-07:00", "notes": ["Facing 297°, west-north-west"], "photo_url": "/api/photo/b81e44d0.jpg" }
  ],
  "access": [
    { "building_id": "costar:1234567", "building_name": "Example Tower", "contact": { "role": "leasing", "name": "Jane Doe", "company": "Example Realty", "phone": "+1 213 555 0100", "email": "jane@example.com" }, "ask": "Roof access 19:00–20:30 on 24 June for a 3-person crew, no lights" }
  ],
  "caveats": ["Height map is from April 2026; newer buildings may be missing"],
  "model": "mock",
  "steps": [ { "tool": "search", "summary": "Scored 1,600 candidates in 300 m around Downtown LA; 10 spots" } ]
}
```

| Field | Meaning |
|---|---|
| `title`, `summary` | Short headline and 1–3 plain sentences for the top of the sheet. |
| `brief` | The `Brief` the agent actually searched with, so the user can see how it read the text. |
| `timeline` | `{t, what}` in time order: arrival, golden hour, the moment of the shot, sunset. |
| `spots` | `role` is `"primary"` (exactly one, first) or `"backup"`. `spot` is a full `Spot`. `arrive_by` is a time. `photo_url` (or `null`) is an image from `/api/photo`. |
| `access` | Who to ask for each roof spot. `contact` is a CoStar `Contact` (or `null` if CoStar has none). `ask` is a ready-to-send one-line request. |
| `caveats` | Honest limits that apply to this sheet. |
| `model` | Which model wrote it (e.g. a Claude model id such as `claude-sonnet-5-5`), or `"mock"`. |
| `search` | The `SearchResult` the spots came from (heat, layers, stages, up to `max_spots` spots), so the app can draw the map and re-weight goals after a chat. Left out by older servers. |
| `steps` | What the agent did, in order: `{tool, summary}` with `tool` one of `read`, `plan`, `search`, `photo`, `buildings`, `sun`, `write`. Show it while/after the agent works. May be empty (added 11/14; optional when parsing). |

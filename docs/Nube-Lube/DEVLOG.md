11/14/26, 11:16AM - Took over Ray's parts (vexcel, server). Built `crates/vexcel`: `get_dsm(lat, lon, radius_m) -> DsmGrid`. Branch `vexcel/dsm`.

## What it does, in plain words
1. Works out which square tiles of Vexcel's height map cover the area (fine tiles near the point, coarse further out).
2. Downloads only the tiles not already in `cache/vexcel/dsm/` (4 at a time, waits and retries if Vexcel says "too many requests").
3. Opens each tile (a GeoTIFF), and for every 1 m × 1 m cell of our output keeps the **highest** point inside it.
   Highest, not average, because a roof edge anywhere in the cell is what blocks the sun.
4. Returns a north-up grid of heights in ground metres, plus where it came from (collection, capture dates).

## What I measured on the live API (downtown LA, Pershing Square)
- `buffer-meter` is a **radius in ground metres**: buffer 30 gives a 60 m box (970 px, 2.7 MB). DESIGN.md's "60 m box" = buffer 30.
- Pixels are 0.0746 EPSG:3857 units = **~6.2 cm on the ground** at LA's latitude (3857 stretches by 1/cos(lat) ≈ 1.21).
  Vexcel's metadata says the photos themselves are 8–15 cm (`min-gsd`/`max-gsd`), so 6 cm is the grid, not the camera.
  So "7.5 cm as advertised" is true for the grid in 3857 units; on the ground it's a bit finer.
- Full detail up to **~8192 px** across. 7952 px stayed fine; asking for more halves the detail (0.1493 units ≈ 12 cm).
- `POLYGON(...)` requests work and snap to one global pixel grid (`pixel-bbox` in the XMP). The world is exactly 2^29 px wide
  at full detail. That's why tiles line up and can be cached by tile number.
- GeoTIFF: float32, big-endian, Adobe deflate, 256 px internal tiles, PixelIsArea, nodata `-9999`, tiepoint at the image centre.
  The ~10% zeros I first saw are the padding of internal tiles past the image edge, not real data.
- Heights are about **above sea level**: ground 79–90 m, roofs to 142 m in the 60 m box; 315 m (a Bunker Hill tower) in 200 m.
- Collection `n34w118-us-ca-losangeles-2026`, captured 19–29 April 2026. No rate-limit headers seen.

## Why the two tile sizes
- A horizon needs 1–3 km around the spot. 1 km radius at full detail is ~1000 fine tiles, ~3 GB. Too much.
- At 1 m output cells, 12 cm source is plenty far away (at 1 km a metre is 0.06°; the sun is 0.53° wide).
- Fine tiles: 1024 px (~63 m, ~3 MB) within 150 m of the point. Coarse tiles: 4608 px (~570 m, ~55 MB) beyond.
  4608 because twice that is over 8192, which forces Vexcel to give us the coarse level (4x fewer bytes per area).
- Checked: a coarse tile came back at exactly 4608 px / 0.1493, aligned to the global grid.

## Results
- 60 m radius: 9 fine tiles, 3.4 s first time, 0.35 s from cache (no key needed then). 120 × 120 grid.
- 200 m radius: 25 fine + 4 coarse tiles, 22 s, 293 MB cached. 400 × 400 grid, no visible seams.
- `cargo test --workspace`: 33 pass. Live test (`cargo test -p vexcel --lib -- --ignored tests::live_downtown_la`) passes.

## Phone (iPhone 13 Pro) notes for later
- None of this runs on the phone. Raw tiles (3–55 MB) never go to the phone; the server sends small grids.
- A 1 km radius at 1 m is 2000 × 2000 = 16 MB of f32: too much for one phone view. For the 3D view, use coarser cells far away
  (`DsmOptions.cell_m`) and send heights as u16 decimetres (half the size). Keep textures ≤ 4096 px.
- First fetch of a 1 km area is ~1 GB of downloads (cached after). Pre-fetch the demo area before the demo.

## Decisions to check with the team
- `DsmGrid` Rust type lives in `crates/vexcel` for now. `core` can't depend on `vexcel` (IO crate), so core should make its own
  plain struct with the same fields, or we move it to a tiny shared crate later.
- Fixture is synthetic (two towers with a gap, a roof to stand on), not real Vexcel data, to keep their licence out of git.
- The `.env` with the key lives in `env/`, which is gitignored. Point `VEXCEL_ENV_FILE` at it.

11/14/26, 11:22AM - Moved the DsmGrid type into a new shared crate, `crates/contracts`.
- Why: `core` must stay pure and can't import `vexcel` (an IO crate), but it needs the DsmGrid type. Now both depend on `contracts`.
- `contracts` is pure: the structs, the lat/lon to cell formula, and turning a grid into JSON + f32 bytes and back.
- The actual file reading/writing stayed in `vexcel::grid` (`read_files`, `write_files`), because IO belongs in the IO crates.
- Regenerated the fixture: byte-for-byte identical. Tests: 7 in contracts, 29 in vexcel, all pass. Cached fetch still works.
- README: documented `VEXCEL_ENV_FILE` and `VEXCEL_CACHE_DIR`, filled in the height map date, added `crates/contracts` to the repo map.
- New team rule (AGENTS.md on main): the agent no longer commits. It leaves changes uncommitted and suggests the commit.

11/14/26, 11:42AM - Filled in every endpoint in `docs/contracts.md`: sun, horizon, search, scene, photo, buildings.
- Each endpoint has its parameters (with defaults and limits), its response shape, a JSON example, and a fixture in `fixtures/`.
- Every shape is also a Rust struct in `crates/contracts`. `cargo test -p contracts` checks every fixture parses into its struct,
  so the doc, the fixtures and the code can't drift apart without a test failing.
- Time zones (Ray's open question): every endpoint that returns times takes `tz` (default America/Los_Angeles) and returns local
  times with the offset, e.g. 20:07:41-07:00. Ray's `/api/sun` on branch `server/sun` returns UTC only, so it needs updating.
- Errors keep Ray's shape `{"error": "..."}`. Status codes: 400 bad input, 404 unknown id, 422 impossible (polar / no data),
  502 Vexcel failed, 503 no key and not cached.
- Phone budget built into the contract: JSON < ~200 KB; 3D heights come as a separate binary of u16 decimetres in 3 rings
  (fine near, coarse far): ~400 KB on a phone instead of 16 MB; photos 1024 px by default, at most 2048.
- "Spot" and "HorizonProfile" share one `sun` block (when the buildings hide the sun, and minutes of golden hour in view).
- Still TODO: CallSheet (the agent's output).
- Next: merge Ray's `server/sun` branch (it adds `crates/core` + `crates/server` and its own root Cargo.toml, so expect conflicts),
  then make `/api/sun` match the contract.

11/14/26, 11:57AM - Contract finished: `POST /api/agent` (`AgentRequest` -> `CallSheet`), the last TODO shape.
- CallSheet = title, summary, the Brief it searched with, a timeline, primary + backup spots, who to ask for roof access
  (with a ready-to-send one-line request), caveats, and which model wrote it ("mock" offline).

11/14/26, 11:57AM - `vexcel::get_oblique(lat, lon, az_deg) -> JPEG bytes` (angled aerial photo of the view).
- How I found the API: Vexcel publishes an OpenAPI spec at https://api.vexcelgroup.com/v2/api-docs (no key needed).
  `/v2/oriented/extract` with a point needs `bbox-dimensions`; without it you get a misleading 404 "No image matching".
- Obliques come in 4 directions (`product-type=oblique-north/east/south/west`). **The name is where the camera looks**: checked with
  `/oriented/query` (camera ~3 km east of the image centre for oblique-west) and by eye (Pershing Square is at the far edge
  looking west, the near edge looking east). So we pick the direction nearest the azimuth asked for: sunset in LA -> west.
- Newest oblique flight over DTLA: `us-ca-losangeles-2025`, shot 1 Dec 2025 (the DSM is April 2026).
- Request: 300 x 200 m around the point, resampled to 1024 x 683 px JPEG (~400 KB, ~1 s), newest flight first, then the photo
  whose centre is nearest the point (least distorted). No EXIF rotation, so it displays upright as-is.
  Vexcel's copyright line is burned into the image; leave it (licence).
- Cached at `cache/vexcel/oblique/<facing>/<lat>_<lon>_<view>_<size>.jpg`; lat/lon rounded to ~1 m so nearby calls share it.
  Second call: 1 ms, no key needed. Max 2048 px per side (phone budget).
- Tests: 5 offline (direction rounding, query, validation, cache key, cache hit without key) + 1 live (ignored by default).
- Try it: `cargo run -p vexcel --example fetch_oblique -- 34.049 -118.251 297` -> `cache/vexcel/out/oblique.jpg`.
- Not done: capture date for `PhotoMeta.captured` (extract doesn't return it; needs an extra `/oriented/query` call) and ortho.

11/14/26, 12:28PM - App design (v2) in DESIGN.md, and a tested map prototype.
- **Vexcel as the map**: `vexcel::ortho::OrthoTiles` fetches XYZ photo tiles (`/v2/ortho/tile`), caches them, and also caches
  "no imagery" answers. Server proxies them at `/api/tiles/vexcel/{z}/{x}/{y}` (204 = none) so the key never reaches the phone.
- **Does it work everywhere? No.** Our key is geofenced: greater LA and greater Houston only. Princeton, NYC, Philly, Chicago,
  San Diego, Austin, Kansas, London: 403 Forbidden (even the "what imagery exists here" call). So the app uses a free global
  basemap (OpenFreeMap) and lays Vexcel photos on top where we have them (zoom 12+; below that Vexcel tiles are black outside coverage).
- **Shading**: sample the sun's elevation on a ~24 x 50 grid of screen points, tint by it (gold / blue / navy), stretch over the map.
  Zoomed out you see the sunset line sweep across the continent; zoomed into a city it's uniform, which is physically right.
- **Prototype**: `/proto/map` (crates/server/static/proto_map.html), MapLibre 4.7.1 + OpenFreeMap + Vexcel + shading + sun/sunset
  lines + 0-24 h slider. Tested in headless Edge emulating an iPhone 13 Pro (390x844 @2-3x, touch) via the DevTools protocol:
  LA golden hour / night with Vexcel photos, Princeton with basemap only, North America zoomed out. Loads in ~5 s cold.
  Bugs found and fixed while testing: times in the browser's zone (now the place's `tz`), basemap 3D buildings drawn over the
  Vexcel photo (hidden now), "next sunset" not saying it was tomorrow's.
- Found Vexcel's textured 3D city mesh (`/v2/3d/tileset`, LA 2020-21, Houston 2023): good base for "street view" before splats.
- **Weather**: Open-Meteo (free, no key, low/mid/high cloud split, aerosols). Tested for Princeton.

11/14/26, evening - Fallback map, photo-match design, sponsor plan
- **Fallback aerial map wired in** (`crates/server/static/proto_map.html`): Esri World Imagery under the Vexcel layer, so
  outside LA/Houston (Vexcel answers 204) you still get an aerial photo. Tested on an emulated iPhone 13 Pro: Princeton z17
  shows Esri's campus photo, downtown LA shows Vexcel on top. Added a Photo / Map button (`?view=map`).
  Checked others: EOX Sentinel-2 works but is 10 m/px; USGS imagery tiles 404.
- **Dropped Peri.** DESIGN.md prize section rewritten: deepagents = yes, plus MLH Gemini, MongoDB Atlas, Vultr, GoDaddy
  (HTTPS needed for camera/location on iPhone), DEV post. Skip Solana, Arlequin, Physical AI.
- **Photo match designed** (DESIGN.md "Photo match"): upload a photo → Gemini + deepagents find where and when to retake it.
  Tools own the numbers; the 3D sim (`/api/render`, same three.js view in headless Chrome) is the agent's eyes; skyline fit
  and sun inverse search in `core`; Vexcel `/v2/embedding/extract` similarity search for candidates; benchmark of Gemini alone
  vs Gemini + tools on geotagged LA photos.
- Gemini key goes in `env/.env` as `GEMINI_API_KEY` + `GEMINI_MODEL` (README rows added). Model ID not verified yet.

11/14/26, night - Height map backup for Princeton: USGS 3DEP LiDAR (`crates/lidar`)
- Task: time-boxed Vexcel backup for heights; must give the same `DsmGrid`. Checked data before building:
  - 3DEP bare-earth DEM works but has no buildings (useless for a skyline on its own).
  - OSM buildings with heights: Overpass failed (406, then 502). Patchy anyway.
  - **3DEP LiDAR point clouds (EPT on AWS): Princeton covered** by DE_DelawareValley_HD_2015. Prototype in Python over
    Nassau Hall: 36 tiles, 20 MB, 630k points, 99.6% of 1 m cells filled, buildings and trees clearly there. Chosen.
- Built `crates/lidar`: pure `ept` (octree keys, walk plan), `index` (pick newest project covering a point), `grid`
  (points -> max per cell, contract formula, noise dropped, pinholes filled); IO `fetch` (cache under `cache/lidar/`,
  backoff on 429/5xx); `get_dsm` with full detail within 150 m, ~1 m points beyond. LAZ decode via the `las` crate.
- Server: `heights::get_dsm` tries Vexcel, falls back to LiDAR on 403 / no key / mostly-empty. Unit tests for the
  decision; live test: Princeton grid -> `core::horizon` works unchanged.
- Measured: 1 km radius = 2000x2000 cells, 23 s cold, ~100 MB cached, 99.9% filled.
- Docs: DESIGN (why LiDAR not OSM, numbers, limits), contracts (`provider: "usgs-3dep"`), README (crate, LIDAR_CACHE_DIR).
- Next per plan: the agent (brief -> tools -> CallSheet, offline mock), then /api/photo and /api/scene.

11/14/26, night - OpenStreetMap out (too low quality): Esri basemap, Google place search
- Map prototype: OpenFreeMap (OSM vector tiles) replaced by Esri raster tiles, no key. Map mode = World Street Map;
  photo mode = Vexcel over Esri World Imagery, with Esri road + place labels on top (names down to "Nassau Hall").
  Tested on emulated iPhone 13 Pro: Princeton map + photo, LA photo. Raster labels are a bit soft at 3x.
  Note: Esri's credits still mention OSM for some areas; fully OSM-free would be Google Map Tiles API (key + billing).
- Photo-match design: Nominatim/Overpass replaced by Google: Gemini Grounding with Google Maps + Places API (New) Text
  Search for coordinates, Place Photos (ground-level, same kind of photo as the user's). Key: GOOGLE_MAPS_API_KEY;
  no key -> Esri World Geocoder. README row added.

11/14/26, late - Server endpoints + agent built, frontend wired, Vexcel covers Princeton
- Vexcel now serves Princeton (n40w74-us-nj-trenton-2026, June 2026): DSM, obliques, tiles. LiDAR stays the backup.
- Far-field DSM tiles fetched at Vexcel `zoom=17` (~0.9 m = our coarse level): 1 km grid ~700 MB -> ~56 MB.
- Server: /api/sun, /api/horizon, /api/search, /api/scene(+bin), /api/photo(+jpg), /api/buildings, /api/agent; grids
  cached in memory; local times via chrono-tz; errors as ApiError JSON. Serves web/ at /.
- Agent: read -> search (re-plan) -> sun -> photo -> buildings -> write. Gemini optional (times validated against the
  tools); offline mock otherwise. Contract change: `CallSheet.steps` (optional), fixture updated. Fixed the sun_path
  fixture (its points contradicted its own sunset).
- Frontend (teammate's web/, asked for by me): adapters from the contract shapes to what the page draws; the brief now
  goes to the agent; map uses real Vexcel/Esri tiles fitted to the searched area; aerial photo via PhotoMeta with
  attribution; 3D view from scene ring 1 with the eye lifted above its own cell; hero 3D city from /api/scene
  instead of Overpass/OSM. Mock fallbacks kept.
- Tested headless at Princeton: hero buildings in 2–5 s, agent sheet in 15–20 s, skyline + 3D + Vexcel oblique live.
- Not done: CoStar kit not on this machine (contacts empty), Gemini and Google Maps keys not in yet, photo `kind=ortho`.

11/14/26, late - Realistic 3D view (web/js/realview.js)
- User: the 3D view was grey and unrealistic. Asked to use Vexcel's Princeton mesh: checked `/v2/3d/collections` and the
  tileset: only LA and Houston for our key (Princeton 3D: 404/403). Told the user; built from heights + photos instead.
- First try (blocks per cell) looked like columns; switched to a 1 m flat-shaded surface with the Vexcel photo draped,
  facade-with-windows shader on steep faces, trees found from DSM bumpiness (photo greenness alone failed: canopy sides
  sample the ground beside the tree), median filter, physical sky + shadows + haze + night windows.
- Wired into the current web/index.html (old simple view stays as the offline fallback). Suu's web/map branch not
  supported (user: outdated, and it brings OSM back).

11/14/26, late - Vexcel 3D mesh in the 3D view (LA/Houston) - HANDOFF, needs a faster machine to finish
- User: the height-map 3D view is too low quality; wants Google-Maps-like realism, using Vexcel wherever possible
  (sponsor). Vexcel's textured mesh (3D Tiles) exists for our key only in LA (Downey 2020-21) and Houston 2023;
  Princeton has none (checked `/v2/3d/collections`). No GOOGLE_MAPS_API_KEY in env/.env yet, so Google 3D tiles untested.
- Mesh data is good: tree goes to zoom 20 (geometric error 0.75 m), 1024-2048 px KTX2 textures per tile (~0.15-0.3 m/px).
- Built (compiles; release server ran with it; `cargo test -p vexcel mesh` 5/5 passed):
  - `crates/vexcel/src/mesh.rs`: cached pass-through (`cache/vexcel/mesh/`), `rewrite_json` (absolute URLs -> proxy;
    adds `tileset.json` / `tile.glb` extensions because the viewer picks loaders by extension), `strip_ext`,
    `trim_tileset` (drop children outside a radius), `covering` (collection at a point).
  - `crates/server/src/mesh.rs`: `GET /api/mesh/at?lat&lon` -> {collection, tileset_url, attribution} or 404;
    `GET /api/mesh/near/{lat}/{lon}/{r}/{tileset.json|tile.glb}?...` (trimmed to r = 3000 m; without the trim the
    viewer loaded coarse tiles for the whole LA basin first). Wired in main.rs.
  - `web/js/meshview.js`: three r160 + 3d-tiles-renderer 0.3.39 (import map in index.html), Draco + KTX2, mesh placed
    in the spot's ENU frame, eye = raycast to the mesh + 1.7 m, unlit photo graded by sun elevation, physical sky.
  - `web/index.html` build3D: `/api/mesh/at` -> meshview, else realview.js (height map + Vexcel photos, Princeton).
- Tested headless (software GL, slow CPU): tiles stream, ground found (~57 m at 34.0505,-118.2545), 0 failed tiles.
  NOT yet seen on screen: my test harness mounted the view off-screen (mount() sets the host to position:relative).
- TODO for whoever picks this up:
  1. Run `cargo test --workspace`, `cargo run --release -p server`, open the app, brief "sunset between two towers from
     a rooftop in downtown LA tonight", open the 3D tab. Check it looks right (orientation: x east, y up, z south).
  2. Tune `tiles.errorTarget` (now 6) and the lruCache sizes for phones; check memory on an iPhone 13 Pro.
  3. Grading/sky in meshview `setSun` was not tuned visually yet.
  4. Houston: same path, untested.
  5. Princeton stays on realview.js unless Vexcel releases a mesh (ask mentors for the collection name) or a Google
     key with Map Tiles API is added (3d-tiles-renderer has GoogleCloudAuthPlugin).
  6. Update docs/contracts.md with /api/mesh/* (not done: contract change goes in its own small PR).

## 11/14/26, 11:19PM: cost-function library, object sources and agent tools (branch `server/cost-library`)

Judges asked for an agent that designs its own cost functions and keeps a library of them. Decision: the agent
writes core's JSON expressions (docs/metrics.md v2), not words and not code. Words can't be evaluated without a
model call per point; code can't be made safe in time. Expressions are checked before use, can't loop or crash,
run in Rust inside the funnel, and only the ones a brief names are evaluated, so a big library can't slow or
confuse a search.

Built on core/metrics (task 051, merged into this branch until it lands on main):
- `crates/vexcel/src/detect.rs`: `POST /v2/elements/detect` over fixed-lattice 300 m boxes touching the area,
  at most 9 per call, nearest first. Each answer is split by label and cached as its own file, so "tree" after
  "tree, car" is free and cached areas work without a key. POST shares the GET backoff on 429/5xx.
- `crates/server/src/objects.rs`: `find_objects(area, kinds, key)` merges CoStar buildings (top = ground + CoStar
  height, else the DSM) and Vexcel detections (top = highest surface in the box) into `contracts::Object`, keeps
  the same thing once (CoStar first, then confidence), at most 60 per kind.
- `agent/library.rs`: seeds from core (read-only) + saves in `cache/library/<name>.json`. Names `[a-z0-9_]`, no
  shadowing a seed or replacing a save, every file re-checked on load and skipped (logged) if bad.
- `agent/trial.rs`: test run on ~50 even points through core's own pieces; a test checks it agrees with
  `search_with`. Refuses a save only if the must drops everything, or nothing is dropped and scores are flat.
  First version also refused "gate drops 75%, kept points all 1.0", which is a good function: fixed, with a caution.
- `agent/tools.rs` + `claude.rs::tool_turn`: the search step as a Claude tool loop (list, define + trial, save,
  find_objects, search, investigate with oblique photos, finish with the agent's order). Strict tools,
  `tool_choice` auto, medium effort, thinking blocks passed back unchanged, tools + system prompt cached.
  Caps: 10 turns, 16 tool calls, 4 defines, 120 s. Refusal / cut-off / HTTP error / never searched / no spots:
  the old fixed search runs and the steps say why. Every tool call is in stderr and the sheet's steps.
- `api::run_search` now searches with seeds + saved functions, so `/api/search` and the chat can `use` them.
- Hook in `agent/mod.rs` is one line (search_with_replan -> tools::search_step) to stay clear of agent/chat (054).

Tests (offline, no keys): fake Claude scripted per turn, synthetic city, the probe's detection sample. A wrong
define comes back with core's error verbatim, the right one with its distribution, it's saved, searched, the
agent re-ranks after investigate, and a later plain /api/search reuses it. Plus: degenerate save refused, refusal
and runaway loops stop, a loop that never searches falls back, bad tool input is an error result.
`cargo test --workspace` green.

Not done / for others:
- Live run not done: no ANTHROPIC_API_KEY on this machine (server says "Claude: no key"). Needs the key in
  env/.env (`ANTHROPIC_API_KEY=...`) or a bare-key file in ANTHROPIC_ENV_FILE, then one LA hero brief.
- Object features (dist, bearing, visible) aren't in core yet (task 052), so functions can't use objects; the
  prompt says so and find_objects only informs the plan for now.
- "fountain" isn't in contracts::OBJECT_KINDS: adding it is a one-line contracts PR (with a detection check).
- trial.rs should move into core as `search::sample` (pure); raised for burvy.
- Detection may be billed separately by Vexcel (probe caveat); boxes are cached and capped, untested above 150 m.

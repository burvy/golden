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

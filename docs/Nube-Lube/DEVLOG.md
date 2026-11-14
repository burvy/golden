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

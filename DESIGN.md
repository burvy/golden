# golden: design

**One line:** a filmmaker types a shot brief ("sun setting between two towers, from a rooftop, late June").
golden returns the best spots in an LA area. Click one to see the real skyline from there with the sun's
path, Vexcel's aerial photo of the view, and the CoStar building with who to ask for roof access.

**Pitch:** PhotoPills tells you where the sun is. golden tells you where to stand, and shows you the shot,
in a city full of buildings.

HackPrinceton F26, 24 h. Tracks: Entertainment (main), CoStar "AI for the Built World" (needs 2 of
Vexcel / CoStar / Matterport, plus an agent), Amazon Practical AI, Peri Forward Deployed (needs a real
business or club consulted, e.g. a campus film club).

## What we offer that PhotoPills doesn't
PhotoPills already has sun lines, AR and alignment search ("Find"). Don't pitch those as new.
1. **Buildings in the horizon:** a 7.5 cm Vexcel height map, so "the sun drops behind that tower at 16:41" is exact.
2. **Search:** scan a whole area for spots that fit a brief. PhotoPills only checks a spot you already picked.
3. **Preview from your desk:** a 3D skyline from the spot with the sun path, next to the real aerial photo.
4. **Getting in:** CoStar roof height, floors, manager/leasing contact, and Matterport interiors.
5. **An agent:** brief in, call sheet out (spots, times, backups, reasons).

## Parts
| Part | Folder | Owner | What it is |
|---|---|---|---|
| core | `crates/core` | burvy | **Pure** functions: coordinates, sun position, horizon from a point, spot scoring and search. No network, no files. |
| vexcel | `crates/vexcel` | CS | Height map (DSM) fetch, disk cache, stitch and downsample to ~1 m; aerial/oblique photo of a view. |
| costar | `crates/costar` | Kinesiology | Load the CoStar kit (local JSONL), find buildings near a point: height, floors, contacts, Matterport ids. |
| server | `crates/server` | CS | axum HTTP API on `localhost:8787` that wires the parts; the agent (Claude tool loop, offline mock model). |
| web | `web/` | Frontend | Map (MapLibre + deck.gl), brief box, spot heatmap, spot card, 3D scene (three.js). |
| pitch | `docs/` | Kinesiology | Demo script, Devpost, film-club interview notes. |

Data flows one way: `vexcel` + `costar` → `core` (pure maths on plain data) → `server` (HTTP + agent) → `web`.
`core` never imports the IO crates. The IO crates turn the outside world into plain structs, and `core` does the thinking.

## Contracts
`docs/contracts.md` is the source of truth for every JSON shape and endpoint. Change it in its own small PR
that everyone sees before code depends on the change. Each shape has a sample in `fixtures/`, so every part can be built
and tested without the others.

Shapes (fields to be fixed in contracts.md): `Brief`, `DsmGrid` (meta + raw float32 metres), `HorizonProfile`
(`[{az_deg, elev_deg}]` from an observer), `SunPath` (`[{t, az_deg, elev_deg}]`), `Spot` (lat, lon, height_m,
score, sun_visible_until, why), `Building`, `CallSheet`.

Conventions: angles in degrees, azimuth clockwise from true north; heights in metres above the DSM datum;
times in ISO 8601 with offset; lat/lon WGS84 in the API; EPSG:3857 only inside `vexcel`.

## Data facts (verified with our key)
- Vexcel: `GET https://api.vexcelgroup.com/v2/dsm/extract?wkt=POINT(lon lat)&buffer-meter=N&srid=4326`,
  header `X-API-KEY`. Returns a GeoTIFF: 1 band, float32 metres, deflate, EPSG:3857. A 60 m box is 7.5 cm (2.5 MB).
  A 600 m box auto-drops to 15 cm (66 MB). So fetch small, cache on disk, downsample.
- Works in LA (collection `us-ca-losangeles-2025`) and Houston. Princeton is geofenced (no data).
- Obliques: `/v2/oriented/collections` then `/v2/oriented/extract` (layer `urban`). Ortho: `/v2/ortho/extract`.
- Roof heights per structure: `/v2/property/extract?include=calculated` (slow).
- CoStar kit: 3,339 commercial properties (1,581 LA) with lat/lon, `buildingHeightInFeet`, `numFloors`,
  Matterport ids. Listings carry contacts. **The licence is unclear, so keep the kit out of git.**
- Sun: NOAA formulas (Equid's `prep-shade/src/sun.rs` can be copied).

## Reuse from Equid (Z:\3Projects\Equid)
Copy, don't depend: `prep-shade/src/sun.rs` (sun), `server/src/{agent,llm,mock,limit,vexcel}.rs` (Claude loop,
offline mock, Vexcel client), `web/` setup (Vite + TS + MapLibre + deck.gl), `prep-shade/src/raster.rs` (grid cleanup ideas).

## Risks
- Height-map volume: horizons need 1–3 km of context, at about 1 m, so cache aggressively and keep the demo area small.
- Scope: one hero flow in one city (downtown LA). Feature freeze at H14.
- Integration: parts meet late, so contracts and fixtures come first, and we run end to end by H10.

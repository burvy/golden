# golden: design

**One line:** a filmmaker types a shot brief ("sun setting between two towers, from a rooftop, late June").
golden returns the best spots in an LA area. Click one to see the real skyline from there with the sun's
path, Vexcel's aerial photo of the view, and the CoStar building with who to ask for roof access.

**Pitch:** PhotoPills tells you where the sun is. golden tells you where to stand, and shows you the shot,
in a city full of buildings.

HackPrinceton F26, 24 h. Tracks (details in "Prize tracks" below): Entertainment (main), CoStar "AI for the Built World" (needs 2 of
Vexcel / CoStar / Matterport, plus an agent), Amazon Practical AI, Peri Forward Deployed (needs a real
business or club consulted, e.g. a campus film club).

## The app (v2, 11/14, draft: will be updated)
**What it is:** a phone mapping app that helps anyone (tourists, photographers, people out for a walk) find a
good place to watch or shoot the sunset. You scroll a map like Google Maps; it shows where the sun will go
down (the sunset line), a Vexcel bird's-eye photo map, and a "street view" that lets you see the spot, with
the sun, before you go. The filmmaker brief and agent above become one feature ("find me a spot for...").

**Target device: iPhone 13 Pro** (A15, 6 GB RAM, Safari / WebKit, 390 × 844 pt screen at 3×). If it runs
smoothly there, it runs on most phones. It is a mobile web app (no App Store), so everything below is
WebGL in mobile Safari.

### Screens
1. **Map** (home): scroll and zoom anywhere. Day/night shading, sunset line, time slider. Tap a place → spot card.
2. **Spot card**: sunset time, when buildings hide the sun from there, golden-hour minutes, weather/cloud
   outlook, the Vexcel oblique photo looking toward the sunset, and who to ask for roof access (CoStar).
3. **Street view**: stand at the spot and look around at the chosen time, with the sun where it will be.
4. **Find a spot** (brief/agent): "sunset between two towers from a rooftop, late June" → ranked spots on the map.

### Map feature (tested 11/14, see "Map: test results" below)
- **Layers, bottom to top:**
  1. Global basemap: OpenFreeMap (free, no key, vector tiles), so the map works everywhere, including Princeton.
  2. Vexcel aerial photos (ortho tiles) on top, wherever Vexcel gives us imagery. The phone gets them through our server
     (`/api/tiles/vexcel/{z}/{x}/{y}`), never with the key. Where there is none, the server answers 204 and
     the basemap shows through.
  3. Day/night shading (below).
  4. Sun line (where the sun is now, yellow) and sunset line (where it will set, dashed orange) from the map centre.
- **Day/night shading:** for a grid of points on the screen we compute the sun's elevation at that place and
  time (NOAA formulas, same as `core::sun`) and tint the map by it: none in daylight, warm orange in golden hour
  (sun 0–8°), blue in blue hour (0 to −6°), dark navy at night (below −18°). The grid is stretched smoothly over
  the screen and redrawn on every pan, zoom and slider move (~1,000 sun calculations, trivial on the A15).
  - **What you'll see:** zoomed out (a country or continent), the dark edge sweeps across the screen as the
    slider moves: golden band, then blue, then night. **Zoomed in to a city it looks uniform**, because the
    sunset line moves at over 1,000 km/h: across a 10 km city the sun's height differs by under 0.1°. So in a city
    the whole map dims together, which is physically right. To make city zoom interesting, add **building
    shadows** from the Vexcel height map later (LA/Houston).
- **Time slider:** now → +24 h in 10-minute steps. Moves the shading, the sun line, and the times on the panel.
  Times are shown in the place's own time zone (`tz`, see docs/contracts.md), not the phone's.
- Prototype: `cargo run -p server`, then open http://localhost:8787/proto/map
  (`?lat=..&lon=..&z=..&tz=America/Los_Angeles&t0=2026-06-21T19:30:00-07:00&h=2` to jump somewhere).

### Map: test results (11/14, headless Edge emulating an iPhone 13 Pro, live Vexcel)
| Where | Vexcel photo tiles | Result |
|---|---|---|
| Downtown LA, z16 | yes (z0–z21) | Aerial photo + shading + sun/sunset lines. Loads in ~5 s cold, then cached. |
| Princeton, z14 | **no: 403 Forbidden** | Basemap + shading + sunset line (16:43 at 246°, matches Open-Meteo's 16:42). |
| North America, z2 | edge of LA/Houston only | Shading shows the sunset line sweeping across the continent. |

**Vexcel does not work everywhere with our key.** It is geofenced: greater LA (incl. Santa Monica, Pasadena,
Long Beach, Irvine) and greater Houston (incl. Galveston) return imagery; Princeton, NYC, Philadelphia, Chicago,
San Diego, Austin, rural Kansas and London return **403 Forbidden** (access, not "no imagery"). Even asking what
imagery exists at Princeton is 403. Zoomed out, Vexcel tiles are black outside coverage, so we only show the
Vexcel layer from zoom 12 up. **Consequence:** the app works everywhere on the basemap; the bird's-eye photos,
obliques, heights (and so "when do buildings hide the sun") work only in LA and Houston. The demo is in LA.
Ask Vexcel at the hackathon whether the key can include Princeton; if so, nothing in the code changes.

### Street view (not tested yet; plan and risks)
- **Goal:** stand at a spot and see roughly what the camera will see, with the sun in place.
- **Inputs we have (LA/Houston):** the Vexcel height map (6 cm), 4-direction obliques + straight-down photos,
  and **Vexcel's own textured 3D city mesh** (`/v2/3d/tileset`, 3D Tiles; LA collection 2020–21, Houston 2023).
- **Recommended order:** (1) the height map as a 3D mesh coloured by the ortho photo, plus a sky with the sun
  (we have all of this already); (2) Vexcel's 3D mesh via 3D Tiles in three.js (`3d-tiles-renderer`), which
  looks much better from street level; (3) **Gaussian splatting** as the stretch goal.
- **Splatting risks:** training a splat needs many overlapping photos of the scene; aerial obliques give ~5
  views per spot from 3 km up, so street-level detail will be weak or need generative fill. Training takes
  minutes to hours on a GPU, so splats must be made ahead of time for demo spots, not on the phone. Viewing on
  an iPhone 13 Pro is fine at up to ~1M splats (web viewers exist, e.g. Spark / gsplat.js); keep scenes small.

### Weather (sunset quality)
- **Use Open-Meteo** (open-meteo.com): free, no key, worldwide, hourly, 16-day forecast. Crucially it splits
  **cloud cover into low / mid / high**, which is what decides a sunset: high and mid clouds catch the light
  and turn red; low cloud on the western horizon blocks the sun. Also visibility, humidity and rain chance,
  and (air-quality API) aerosol optical depth for haze. Tested for Princeton 11/14: tonight 89% low cloud,
  0% mid/high → a grey sunset, which the data says plainly.
- Call it from the server (one call per ~0.1° area per hour, cached), not from the phone. Free tier is
  non-commercial and ~10,000 calls/day: fine for the hackathon; paid plan or self-hosting later.
- Alternatives: NWS api.weather.gov (US only, free, no low/mid/high split), OpenWeather / Tomorrow.io
  (need keys; Tomorrow.io has good cloud layers but a small free tier).
- **Sunset score idea:** good when mid/high cloud is 30–70%, low cloud near the sunset direction is low,
  no rain, and visibility is high. Show it as "Sunset outlook: great / ok / poor" on the spot card.

### Phone budget (iPhone 13 Pro)
- JSON responses < ~200 KB; heights as a separate u16 binary; photos ≤ 2048 px (default 1024); textures ≤ 4096.
- Map: the shading uses one tiny canvas stretched over the map (no per-frame WebGL work); tiles cached by the
  browser for a week. Street view: ≤ ~200k height cells or ≤ ~1M splats per scene.
- The server binds to 127.0.0.1; to try it on a real phone on the same Wi-Fi it must bind 0.0.0.0 (TODO).

## Prize tracks: what each one needs from us (11/14)
**Must-have: CoStar "Inside and Out: AI for the Built World".** Judged on: data use 30%, agentic depth 30%,
usefulness 25%, demo 15%. Hard rules: at least **two of** CoStar / Matterport / Vexcel, combined so each adds
something; an **agent** that reasons, plans and acts (not a chat wrapper); spatial and visual.

| Requirement | Where we are | What we must still build |
|---|---|---|
| Vexcel | **Done**: height map (`get_dsm`), obliques (`get_oblique`), map tiles, 3D mesh found | Horizon + "when buildings hide the sun" (core) |
| CoStar | **Not started.** Kit is not on any machine we've checked (`COSTAR_KIT_DIR` unset) | `crates/costar`: load the kit, `/api/buildings` near a spot. Then *use* it (below) |
| Matterport | Not started | Stretch: show the inside of the building you'd shoot from (tour link on the spot card) |
| Agent | Contract only (`POST /api/agent`) | The tool loop, with its plan and steps visible in the app |
| Where | Data exists **only for LA and Houston** (Vexcel and CoStar) | **Demo in downtown LA.** Princeton shows the basemap only |

**How the layers combine (this is the 30% "use of data"; it can't be decoration):**
1. **Vexcel height map → which roofs actually see the sunset**, and until what minute buildings hide the sun.
2. **CoStar → is that roof a real building we can get onto**: its height and floors (cross-checked against the
   Vexcel roof height), property type, and **who to contact** (manager/leasing). Available space and leases are
   a bonus: a vacant top floor or an amenity deck is a place you can actually rent for a shoot.
3. **Vexcel oblique → what the view looks like** toward the sunset from there.
4. **Matterport → the inside**: the room or terrace you'd shoot from.
5. **The agent ties them together:** turns the brief into a search, ranks spots, checks the weather outlook,
   drops spots with no access contact, and writes the call sheet with ready-to-send access requests. If the
   weather is poor it re-plans for another day. **Show its steps in the app** (agentic depth = 30%).

**Real-estate angle (helps "usefulness"):** the same engine scores **any CoStar building's sunset view**. When
does the sun drop behind neighbours, from which floor is it clear, and how many golden-hour minutes are there
per season? That lets brokers market "west-facing, sunset view until 19:58 in June", and tells real-estate photographers when to
book **twilight shoots** of a listing. CoStar's judges ask "would someone switch this on Monday?": brokers and listing photographers would.

**Also going for:**
- **Amazon, Best Practical AI:** fits as-is (agentic, real users, simple UX). They also score **adaptive**: the
  agent should remember what the user liked (e.g. prefers "between towers", roofs, weekends). Must ship a working
  demo, a clear target user, basic docs (README) and a pitch.
- **Peri, Forward Deployed:** we must **consult a real business or club during the hackathon** and submit its
  name, the person's name and role, and evidence (screenshot, photo, video or quote). It should be someone who
  could use the app *now*, i.e. who shoots in **LA or Houston** (e.g. a real-estate photographer or brokerage
  there, reached by phone/DM), or a Princeton photo/film club if we're open that their area isn't covered yet.
  **Owner: TBD. Do this early**; their answers should shape what we build.
- **LangChain, Best use of deepagents:** only if we build the agent with LangChain's `deepagents` (planning
  to-do list, sub-agents, scratch files) calling our Rust API as tools. It also makes the agent visibly
  "plan and act" for the CoStar judges. Cost: a small Python (or JS) agent service next to the Rust server.
  **Decision needed.**
- Not a fit: Arlequin (forensics / social networks), Physical AI (hardware).

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
  Re-tested 11/14: our key gets 403 everywhere outside greater LA and greater Houston (see "Map: test results").
- Map tiles: `/v2/ortho/tile?layer=urban,wide-area&zoom=z&tile-x=x&tile-y=y`, standard XYZ, z0–21, 256 px,
  JPEG (PNG at coverage edges). Full API spec (no key needed): https://api.vexcelgroup.com/v2/api-docs
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

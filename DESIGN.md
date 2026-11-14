# golden: design

**One line:** a filmmaker types a shot brief ("sun setting between two towers, from a rooftop, late June").
golden returns the best spots in an LA area. Click one to see the real skyline from there with the sun's
path, Vexcel's aerial photo of the view, and the CoStar building with who to ask for roof access.

**Pitch:** PhotoPills tells you where the sun is. golden tells you where to stand, and shows you the shot,
in a city full of buildings.

HackPrinceton F26, 24 h. Tracks (details in "Prize tracks" below): Entertainment (main), CoStar "AI for the Built World" (needs 2 of
Vexcel / CoStar / Matterport, plus an agent), Amazon Practical AI, LangChain deepagents, and MLH prizes
(Gemini, MongoDB Atlas, Vultr, GoDaddy, DEV). Not going for Peri.

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
5. **Photo match**: upload a photo you love (Pinterest, Instagram) → the agent finds where and when you can take
   one like it, shows the simulated view next to yours, and you can chat with it. See "Photo match" below.

### Map feature (tested 11/14, see "Map: test results" below)
- **Layers, bottom to top:**
  1. Global basemap: OpenFreeMap (free, no key, vector tiles), so the map works everywhere, including Princeton.
  2. **Fallback aerial photo everywhere else: Esri World Imagery** (free, no key, good to ~z19; tested 200 at
     Princeton). Attribution "Esri, Maxar, Earthstar Geographics" is required and shown. Free use is fine for a
     hackathon demo; production needs an ArcGIS account (or switch to EOX Sentinel-2 cloudless, which is free but
     only 10 m per pixel). USGS imagery tiles returned 404.
  3. Vexcel aerial photos (ortho tiles) on top, wherever Vexcel gives us imagery: sharper and newer than Esri.
     The phone gets them through our server (`/api/tiles/vexcel/{z}/{x}/{y}`), never with the key. Where there
     is none, the server answers 204 and the fallback photo shows through.
  4. Day/night shading (below).
  5. Sun line (where the sun is now, yellow) and sunset line (where it will set, dashed orange) from the map centre.
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
- **Photo / Map button** (top left) switches between the aerial photo (Vexcel + Esri) and the plain map
  (with its buildings). `?view=map` starts on the plain map.
- Prototype: `cargo run -p server`, then open http://localhost:8787/proto/map
  (`?lat=..&lon=..&z=..&tz=America/Los_Angeles&t0=2026-06-21T19:30:00-07:00&h=2` to jump somewhere).

### Map: test results (11/14, headless Edge emulating an iPhone 13 Pro, live Vexcel)
| Where | Vexcel photo tiles | Result |
|---|---|---|
| Downtown LA, z16 | yes (z0–z21) | Aerial photo + shading + sun/sunset lines. Loads in ~5 s cold, then cached. |
| Princeton, z14 | **no: 403 Forbidden** | Basemap + shading + sunset line (16:43 at 246°, matches Open-Meteo's 16:42). |
| North America, z2 | edge of LA/Houston only | Shading shows the sunset line sweeping across the continent. |
| Princeton, z17, with fallback | no (204) | **Esri aerial photo** of campus + labels + sun lines. Loads in ~5 s. |
| Downtown LA, z17, with fallback | yes | Vexcel photo on top (Esri hidden under it). |

**Vexcel does not work everywhere with our key.** It is geofenced: greater LA (incl. Santa Monica, Pasadena,
Long Beach, Irvine) and greater Houston (incl. Galveston) return imagery; Princeton, NYC, Philadelphia, Chicago,
San Diego, Austin, rural Kansas and London return **403 Forbidden** (access, not "no imagery"). Even asking what
imagery exists at Princeton is 403. Zoomed out, Vexcel tiles are black outside coverage, so we only show the
Vexcel layer from zoom 12 up. **Consequence:** the app works everywhere on the basemap plus the Esri fallback photo; the bird's-eye photos,
obliques, heights (and so "when do buildings hide the sun") work only in LA and Houston. The demo is in LA.
**Heights now have a US-wide backup** (USGS LiDAR, next section), so horizons and "when do buildings hide the
sun" also work in Princeton.

### Heights outside LA/Houston: USGS 3DEP LiDAR (built 11/14, `crates/lidar`)
- **Why LiDAR, not OpenStreetMap:** OSM building heights are patchy (most buildings have none) and its query
  server failed twice in testing (406, 502). 3DEP is measured, covers most of the US, is public on AWS
  (no key), and gives trees too, which matter as much as buildings for a sunset view.
- **Checked before building:** Princeton is covered by `USGS_LPC_DE_DelawareValley_HD_2015_LAS_2017`
  (41 billion points). Over Nassau Hall: 0.6 m point spacing at the deepest level, 99.6% of 1 m cells filled,
  Nassau Hall's T shape and cupola, single trees and every roof are visible. No "building" class in this survey
  (only ground / unclassified), which doesn't matter: a DSM keeps the highest return per cell.
- **Same output as Vexcel:** `lidar::get_dsm(lat, lon, radius_m) -> DsmGrid`, provider `"usgs-3dep"`.
  `server::heights::get_dsm` tries Vexcel and falls back to LiDAR on 403 / no key / mostly-empty grid, so
  `core` and the endpoints don't change. Tested end to end: LiDAR grid → `core::horizon` at Princeton.
- **Cost:** 150 m radius ≈ 20 MB, 6 s; 1 km radius ≈ 100 MB, 23 s the first time, then cached
  (`cache/lidar/`). Full detail within 150 m, ~1 m points beyond.
- **Limits:** surveys are years old (Princeton 2015: new buildings missing); heights are above sea level
  (NAVD88), Vexcel's are on its own datum, so never mix the two in one grid.
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

### Photo match: "where and when can I take this photo?" (11/14, design, not built)
**The user story:** you saw a photo on Pinterest or Instagram (a sunset between towers, a pier at golden hour)
and want to take one like it. You upload it. The agent works out **where to stand, which way to point, how
high, and on which dates and minutes the sun will be in the same place**, shows you our simulated view next
to your photo, and you can chat with it ("closer to Santa Monica", "not a rooftop", "this weekend only").
Then tap the spot → street view to scout it before you go.

**Why this is not an LLM wrapper.** A chatbot can guess "looks like Santa Monica Pier." Our agent has to
*prove* it: it can open the map, query Vexcel's image search, read the height map, put a camera in our 3D
simulation, render what that camera would see with the sun at a given minute, and compare the render with
the user's photo, then move the camera and try again. Every number it reports (coordinates, heading, height,
times, distances) comes from a tool, not from the model's text. That loop (look → hypothesise → render →
compare → refine) is what a plain chatbot can't do.

**Model:** Gemini (MLH "Best Use of Gemini API"), multimodal: it reads the user's photo and our renders.
The model ID goes in `GEMINI_MODEL` (the user said "gemini 4 hydrogen"; check the exact ID with the models
list once the key is in, see "Keys" below). Harness: LangChain **deepagents** (planning to-do list,
sub-agents, scratch files), in a small Python service next to the Rust server.

#### The pipeline (each step is a tool or a sub-agent; the plan and steps stream to the phone)
1. **Read the photo** (sub-agent `reader`): returns a structured *fingerprint* with a confidence for each field.
   - EXIF first (deterministic): GPS, time, focal length, compass heading. Instagram and Pinterest strip
     EXIF, so usually there is none, but when there is, it's ground truth.
   - Gemini, structured output: scene type (skyline / beach / pier / park / mountain), named landmarks
     and any readable text (signs, shop names), the skyline silhouette as ~50 (x, y) points, horizon line,
     where the sun is in the frame (or the light direction from shadows), its height above the horizon in
     degrees (from the horizon line and the estimated field of view), focal length / field of view,
     camera height (ground, rooftop, drone), and season clues (leaves, clothes).
2. **Find candidate places** (sub-agent `scout`), wide to narrow:
   - **Landmarks:** Gemini with **Google Search grounding** names them; we geocode them (Nominatim or
     OpenStreetMap). Model text alone is never trusted for coordinates.
   - **Vexcel image similarity search** (`/v2/embedding/extract`, LA/Houston): search by `search-text` (for
     example "pier with ferris wheel over the ocean") and by `search-image` (the user's photo or a crop) inside
     the area, `product-type=oblique`. Returns places whose aerial photos look similar. Caveat: the user's
     photo is from the ground and Vexcel's are from the air. **Test early** whether image search works across
     that gap; text search is the safe default.
   - **Map features** (OpenStreetMap / Overpass): viewpoints, piers, peaks, rooftop bars, parks with a
     western view.
   - **CoStar:** buildings at a candidate whose roof height matches the camera height ("shot from about the 20th floor").
3. **Place the camera** (sub-agent `surveyor`): for each candidate, solve where the camera stands and which way
   it points.
   - From the landmark's position and its direction and size in the photo (known field of view) → a first guess
     at the camera position and heading.
   - **Numeric refinement, not LLM guessing:** a small search over position, height and heading that
     maximises how well our **rendered skyline** (Vexcel height map, `core::horizon`) matches the photo's
     skyline points. This is plain maths in `core`, testable with synthetic walls.
   - **Visual check in the simulation** (`render_view`, below): render the best poses and have Gemini compare
     each with the user's photo, side by side, against a fixed rubric (same landmarks? same order left to
     right? horizon at the same height? similar framing?) and give a score. Loop until good enough, or until
     the step budget runs out (then say so).
4. **Find the time** (sub-agent `timer`): we know the camera heading, so the sun's position in the photo gives
   its azimuth and elevation. `core::sun` searches the next 12 months for the minutes when the sun is there.
   Because the sunset azimuth moves with the seasons, this also gives **the season**: "between Apr 20–28 and
   Aug 15–23, 19:42–19:50." Then Open-Meteo for the dates within 16 days, and the sunset outlook.
5. **Answer:** ranked spots, each with pose (lat, lon, height, heading, focal length), the next good dates and
   minutes, weather outlook, **side by side: your photo | our render | the Vexcel oblique**, how sure we are
   (match score, not vibes), and how to get there or get in (CoStar contact for rooftops). Each spot opens in
   street view at that pose and time.

#### The simulation as the agent's eyes: `render_view`
- `POST /api/render {lat, lon, height_m, heading_deg, pitch_deg, fov_deg, time, w, h}` → PNG plus metadata.
  It renders **the same three.js street view the user sees**, in headless Chrome on the server (we already
  drive headless Edge over CDP for our map tests), so the agent sees exactly what the user will.
- In LA/Houston: Vexcel 3D mesh or height map coloured by the ortho photo, with the sun and sky at that time.
- Elsewhere in the US: the **USGS 3DEP LiDAR height map** (`crates/lidar`, ~0.5 m, real roofs and trees; see
  "Heights outside LA/Houston") coloured by the Esri photo. Older (Princeton: 2015) and no obliques, so the
  answer says "lower confidence (no Vexcel data here)". Outside the US: no 3D, map only.
- Also `get_oblique` (real aerial photo looking toward the target) as a second, real-world view.

#### Accuracy and precision: how we make sure it's right
- **Tools own the numbers.** The final answer is assembled by the server from tool results; the model only
  picks and explains. Any coordinate or time in the model's text that isn't from a tool is rejected.
- **Show the evidence:** every spot shows the side-by-side and the match score, so the user can judge it.
- **Says when it doesn't know:** a low score → "best guess, here's why"; no data → "outside our 3D area."
- **Benchmark (do this early, keep it running):** ~20–30 photos in LA with a known place and time (Creative
  Commons photos on Wikimedia or Flickr that have a geotag and timestamp; strip the EXIF before testing).
  Measure the distance error (median metres), the heading error, the sun-time error (minutes), and whether the
  right spot is in the top 3. Run **Gemini alone** against **Gemini plus our tools**. That gap is the pitch slide
  that proves "not an LLM wrapper", and it's the number we try to improve.
- Budgets: max ~25 tool calls and ~12 renders per question, so an answer takes about 1–2 minutes. Stream the
  steps so the wait is visible and interesting (agentic depth is 30% of the CoStar score).

#### Chat
- One thread per uploaded photo. Threads and memory are stored (LangGraph checkpointer in MongoDB Atlas).
  Follow-ups re-plan from the saved fingerprint and candidates instead of starting over.
- The user can upload more photos ("more like this one"), and the agent learns their **taste** (prefers water,
  rooftops, silhouettes) for future suggestions (Amazon's "adaptive").

#### Pieces and owners (proposed; the contracts go in `docs/contracts.md` in their own small PR when we build them)
| Piece | Where | Notes |
|---|---|---|
| `POST /api/match` (photo upload) → `{thread_id}`; `POST /api/match/{thread}` (chat message); `GET /api/match/{thread}/events` (SSE steps) | server (proxy) + `agent/` | Rust server stays the only public entry; it forwards to the agent |
| `agent/` (Python, deepagents + langchain-google-genai) | new folder | Tools = HTTP calls to our Rust API |
| `/api/render` | server + `web/` street view page | Headless Chrome on the server |
| Skyline matching, sun inverse search | `crates/core` | Pure, unit-tested |
| `vexcel::search_similar(text or image, area)` | `crates/vexcel` | Wraps `/v2/embedding/extract`, cached |

#### Keys (Gemini)
Get a key at https://aistudio.google.com/apikey and add two lines to **`env/.env`** (gitignored; don't paste the
key in chat or commit it):
```
GEMINI_API_KEY=your-key-here
GEMINI_MODEL=the-model-id
```
Check the exact model ID, without printing the key: list the models with
`curl -s -H "x-goog-api-key: $GEMINI_API_KEY" https://generativelanguage.googleapis.com/v1beta/models`.
The agent passes the key explicitly to `ChatGoogleGenerativeAI(model=..., api_key=...)`.

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

**Photo match makes the CoStar story stronger:** it uses **Vexcel four ways** (image similarity search to find
candidates, the height map to place the camera, obliques as real views, and the 3D mesh in the simulation)
and **CoStar** to say whether the spot is a real roof you can get onto and who to ask. It is also clearly an
agent that plans, acts, looks, and checks its own work.

**Also going for:**
- **Amazon, Best Practical AI:** fits as-is (agentic, real users, simple UX). They also score **adaptive**: the
  agent remembers your taste from the photos you upload and the spots you save (water, rooftops, weekends).
  Must ship a working demo, a clear target user, basic docs (README) and a pitch.
- **LangChain, Best use of deepagents: yes.** The photo-match agent is a deep agent: Gemini as the model, our
  Rust API as tools, sub-agents `reader`, `scout`, `surveyor`, `timer`; its to-do list and scratch files
  (fingerprint, candidates, renders) are shown in the app. Setup: `pip install deepagents langchain-google-genai`,
  then `create_deep_agent(model=ChatGoogleGenerativeAI(...), tools=[...], subagents=[...])`.
- **MLH, Best Use of Gemini API:** Gemini is the agent's brain and its eyes: it reads the user's photo, uses
  Google Search grounding for landmarks, and compares our renders with the photo. The benchmark (Gemini alone
  vs Gemini plus our tools) is what to show.
- **MLH, MongoDB Atlas:** chat threads and agent memory (LangGraph's MongoDB checkpointer), saved spots with a
  **2dsphere geo index** ("spots near me"), and **Atlas Vector Search** over photo embeddings ("spots that look
  like this photo", built from everyone's matches). One database for all three.
- **MLH, Vultr:** host the Rust server, the agent and headless Chrome on a Vultr instance so judges can open it
  on their phones. Stretch: a Vultr GPU for training Gaussian splats of demo spots.
- **MLH, GoDaddy Registry:** register a domain for the demo. Needed anyway: **iPhone Safari only allows the
  camera, photo library and location on HTTPS**, so the phone demo needs a real domain with a certificate
  pointing at the Vultr box.
- **MLH, DEV Writing Challenge:** a DEV post, "Giving an LLM eyes: geolocating a photo with a 3D simulation",
  with the benchmark numbers. Cheap, and it doubles as the Devpost write-up.
- **SpaceXSI (Cursor + Grok Bot raffle):** a raffle, only if someone on the team actually uses Cursor. No build work.
- **Pick one database:** MongoDB Atlas (above) *or* Tiger Data (Postgres + pgvector + PostGIS, time series for
  sun and weather). MongoDB fits better because of the ready-made LangGraph checkpointer.
- **Not a fit:** Peri (dropped), Solana (nothing here needs a blockchain), Arlequin (forensics / social
  networks), Physical AI (hardware).

## What we offer that PhotoPills doesn't
PhotoPills already has sun lines, AR and alignment search ("Find"). Don't pitch those as new.
1. **Buildings in the horizon:** a 7.5 cm Vexcel height map, so "the sun drops behind that tower at 16:41" is exact.
2. **Search:** scan a whole area for spots that fit a brief. PhotoPills only checks a spot you already picked.
3. **Preview from your desk:** a 3D skyline from the spot with the sun path, next to the real aerial photo.
4. **Getting in:** CoStar roof height, floors, manager/leasing contact, and Matterport interiors.
5. **An agent:** brief in, call sheet out (spots, times, backups, reasons).
6. **Photo match:** upload someone else's photo, get the spot, the heading and the dates and minutes to retake it,
   checked against a 3D render.

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
- Works in LA (collection `us-ca-losangeles-2025`) and Houston. Princeton is geofenced (no data); there we use
  USGS 3DEP LiDAR: index `https://usgs.entwine.io/boundaries/resources.geojson`, data
  `https://s3-us-west-2.amazonaws.com/usgs-lidar-public/<project>/ept.json` (EPT octree of LAZ, EPSG:3857).
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

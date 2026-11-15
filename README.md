# `golden`
Find where to stand, for any shot.

Tell golden the shot you want in plain English ("sun setting between two towers, from a rooftop, tonight").
It asks a question or two, turns your words into goals you can see and edit, searches every standpoint in the area,
and gives back a short ranked list. For each spot you get the sun over the skyline in 3D, a Vexcel aerial photo,
and, in LA, who to call about the building (CoStar).

**Try it:** https://burvy.dev/golden (cached demo areas: downtown Los Angeles and Princeton).
Design: [DESIGN.md](DESIGN.md). Cost-function library results: [docs/library-lab.md](docs/library-lab.md).

HackPrinceton Fall 2026: Entertainment track, CoStar "AI for the Built World".

## Run it

**Prerequisites:** Rust (stable). Nothing else: `web/` is plain HTML/JS served by the server, with no build step. Windows: run the scripts from PowerShell.

```powershell
.\build.ps1 # builds the Rust workspace in release (-Test also runs the tests)
.\run.ps1 # starts the server
```
Then open http://localhost:8787.

Without the scripts (any OS), from the repo root:
```sh
cargo run --release -p server   # serves web/ at / and the API under /api
```
Open http://localhost:8787 (phone on the same Wi-Fi: set `GOLDEN_BIND=0.0.0.0:8787` and open the laptop's IP).
Type a shot into Golden Chat, e.g. "sunset between two buildings from a rooftop in Princeton tonight". The first
search in a new area downloads its heights (~10–30 s); after that it is cached. Keys go in `env/.env` (or point `VEXCEL_ENV_FILE` / `ANTHROPIC_ENV_FILE` at a file holding just the key); the server prints at startup whether it found each one. With no Claude key
the agent runs as an offline mock, and opened as a file (no server) the page shows sample data.

## Test
```sh
cargo test --workspace
```

## Configuration
Set these as environment variables. **Never commit their values.**

| Variable | What for | If missing |
|---|---|---|
| `VEXCEL_API_KEY` | Height maps and aerial photos | Read from `VEXCEL_ENV_FILE`, else the shared `.env`. With none, areas already in the cache still work; new areas fail with a "no Vexcel key" error |
| `VEXCEL_ENV_FILE` | Path to the Vexcel key file: either just the key on one line (as Vexcel hands it out), or a `.env` with `VEXCEL_API_KEY=...`. If `GOLDEN_ENV_FILE` isn't set, it's also the shared `.env` the other `NAME=...` keys are read from. Keep key files out of git (`env/` and `*.env` are gitignored) | The shared `.env` |
| `ANTHROPIC_ENV_FILE` | Path to the Claude key file: just the key on one line, or a `.env` with `ANTHROPIC_API_KEY=...` | The shared `.env` |
| `GOLDEN_ENV_FILE` | Path to the shared `.env` with several `NAME=value` lines (`VEXCEL_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_MAPS_API_KEY`, ...). A bare key is never read from it | `VEXCEL_ENV_FILE`, else `env/.env` (so running from the repo root just works) |
| `VEXCEL_CACHE_DIR` | Where downloaded Vexcel height-map tiles are kept (gitignored). A 200 m area is ~300 MB, a 1 km area ~1 GB | `cache/` in the folder you run from |
| `LIDAR_CACHE_DIR` | Where USGS LiDAR downloads are kept (gitignored). ~20 MB per 150 m radius, ~100 MB per 1 km | `VEXCEL_CACHE_DIR`, else `cache/` |
| `GOOGLE_MAPS_API_KEY` | Photo-match agent's place search (Places API (New): coordinates, place photos). Same `env/.env` | Landmarks via Esri's geocoder, no place photos |
| `ANTHROPIC_API_KEY` | The agent (`POST /api/agent`) reads the brief and writes the call sheet with Claude. Get one at platform.claude.com; put it in `env/.env` or point `ANTHROPIC_ENV_FILE` at it. Never printed or logged | The agent runs as the offline mock (rule reader + templates; every number is the same) |
| `GOLDEN_CLAUDE_MODEL` | Claude model ID for the agent | `claude-sonnet-5-5` |
| `GOLDEN_OFFLINE`, `GOLDEN_CLAUDE*`, `GOLDEN_RATE*`, ... | Public-server guards | See "Going live" |
| `COSTAR_KIT_DIR` | Path to the CoStar data kit (kept out of git) | Building lookups use fixtures |

PowerShell example (current window only):
```powershell
$env:VEXCEL_ENV_FILE = "$PWD\env\vexcel.env"      # just the key, or VEXCEL_API_KEY=...
$env:ANTHROPIC_ENV_FILE = "$PWD\env\claude.env"   # just the key, or ANTHROPIC_API_KEY=...
cargo run -p vexcel --example fetch_dsm -- 34.049 -118.251 60   # lat lon radius_m: checks the key works
```

## Going live (burvy.dev)
A public server must not let visitors start downloads, fill the disk or spend Claude credit. Run it cache only,
behind the guards in `crates/server/src/guard.rs`:

```powershell
$env:GOLDEN_OFFLINE = "1"                          # never download: Vexcel, USGS LiDAR, Google, detection
$env:VEXCEL_CACHE_DIR = "D:\golden-cache"       # the pre-fetched demo areas (Downtown LA, Princeton)
$env:GOLDEN_CLAUDE_DAILY_MAX = "300"               # Claude calls a day for the whole site (or GOLDEN_CLAUDE=off)
$env:GOLDEN_BIND = "127.0.0.1:8787"                # loopback only (the default); nginx proxies /golden/api/ here (burvy.dev uses 8797)
cargo run --release -p server
```

| Variable | What it does | Default |
|---|---|---|
| `GOLDEN_OFFLINE` | `1`: cache only. Cached areas work as usual; anything else is a 503 "only serves cached demo areas". The Vexcel and Google keys are ignored, LiDAR doesn't download | off |
| `GOLDEN_CLAUDE` | `off`: never call Claude; the agent and chat use the offline reader | on (if a key is set) |
| `GOLDEN_CLAUDE_DAILY_MAX` | Claude calls per UTC day, whole server; past it, the offline reader until midnight UTC | 300 |
| `GOLDEN_RATE_AI_PER_MIN` | Per IP: `/api/chat`, `/api/agent` (burst 2x) | 6 |
| `GOLDEN_RATE_HEAVY_PER_MIN` | Per IP: search, horizon, scene, photo (burst 2x) | 30 |
| `GOLDEN_RATE_PER_MIN` | Per IP: every other `/api/*` call (tiles, mesh, `.bin`/`.jpg` and the web app are not limited) | 300 |
| `GOLDEN_HEAVY_CONCURRENCY` | AI + heavy requests running at once; others queue up to 30 s, then 503 | 4 |
| `GOLDEN_TRUSTED_PROXIES` | Client IP for the limits: when the TCP peer is loopback (nginx on the same machine) or one of these IPs (comma list), it's `CF-Connecting-IP`, else the first `X-Forwarded-For` hop; otherwise the peer address, so direct clients can't fake it | loopback only |
| `GOLDEN_CORS_ORIGINS` | Comma list of origins that may call the API from pages on another origin. Not needed on burvy.dev (page and API share the origin) | none |
| `GOLDEN_WARM` | `0`: don't load the preset demo grids at startup (normally ~10 s in the background, so the first search is fast) | on |
| `GOLDEN_BIND` | Listen address (`GOLDEN_ADDR` still works) | `127.0.0.1:8787` |

Over the limit: 429 with `Retry-After`. The startup log prints one `guard:` line with the settings in force.

Disk: offline, the only thing written is `cache/scene/<id>.bin` (one per distinct 3D view, a few hundred KB to a
few MB). Delete `<cache>/scene/` any time to reclaim it; scenes are rebuilt on demand. In memory, at most 4 height
grids and 32 scenes are kept.

## How it works
1. **Height map:** Vexcel's surface model of every roof, tree and street (6 cm tiles, ~1 m coarse), cached on disk.
   Outside Vexcel's area, USGS 3DEP LiDAR fills in.
2. **Sun:** NOAA's formulas give the sun's direction and height for any time and place (matches NOAA's own
   calculator to within 1 s and 0.01 deg).
3. **Chat to goals:** Claude (or an offline reader with no key) turns your words into goals: library cost functions
   such as `sun_between`, `open_horizon` or `in_front`, each a must or a weight. You can edit them in the goals panel.
4. **Funnel search:** every standpoint on a grid (up to 1,600) is checked against the goals, cheapest first.
   A failed must drops the point before the costly skyline trace. Survivors are ranked by the weighted goals,
   and each spot reports a score and a reason per goal.
5. **Library:** cost functions are small JSON expressions over features core computes (distance, bearing,
   line of sight to objects, sun scores). One is kept only if its cheap check never loses a top-10 spot:
   15 kept, 100% recall over 18 places and 3 days, dropping a median 41-93% of standpoints early
   ([docs/library-lab.md](docs/library-lab.md)). With `GOLDEN_PLANNER=1`, Claude also writes, tests and saves new ones.
6. **Results:** the map shows a layer per goal; each spot has Map / Photo / 3D view tabs and, in LA, CoStar contacts.

## Why not PhotoPills?
PhotoPills tells you where the sun is. golden tells you where to stand, and shows you the shot, in a city full of buildings.

## Limits
- Vexcel heights and aerial photos: Los Angeles, Houston and Princeton. Elsewhere in the US, heights come from
  USGS LiDAR (older, coarser) and the map from Esri. The public site serves only its cached areas.
- 3D view: needs a Vexcel mesh, so it works in LA but not yet in Princeton.
- CoStar buildings and contacts: Los Angeles and Houston only, so "who to call" is empty elsewhere.
- Objects the search knows: CoStar buildings, plus Vexcel detections (building, tree, car, rooftop HVAC unit,
  swimming pool, water tower). No water bodies or benches yet.
- Height map date: LA flown 19-29 April 2026 (Vexcel collection `n34w118-us-ca-losangeles-2026`); newer buildings may be missing.
- Computed vs estimated: sun times, skylines and scores are computed from the height map. Photo notes are the
  AI's reading of aerial photos, and are labelled as such. No weather yet.

## Repo map
| Folder | What |
|---|---|
| `crates/core` | Pure maths: coordinates, sun, horizon, scoring, search. No network, no files. |
| `crates/vexcel` | Height map and aerial photo download, cache, downsample |
| `crates/lidar` | Backup height map outside Vexcel's area (USGS 3DEP LiDAR, public, no key), same `DsmGrid` |
| `crates/costar` | CoStar kit loader + building lookup |
| `crates/server` | HTTP API on :8787, the chat and agent, the cost-function library tools, public-server guards |
| `web/` | The page: Golden Chat, goals panel, map layers, spot card with Photo / 3D view |
| `docs/` | Contracts, pitch, Devpost, devlogs (`docs/<name>/DEVLOG.md`) |
| `fixtures/` | One sample file per data shape in `docs/contracts.md` |
| `crates/contracts` | The shapes from `docs/contracts.md` as Rust structs. Pure, so `core` can use it; IO crates fill it |

## How we work
- Read [AGENTS.md](AGENTS.md) first (people and AI agents alike).
- One branch per piece of work: `<part>/<what>`, e.g. `core/horizon`. Open a PR to `main`, which gets reviewed before merge.
- Shared data shapes live in `docs/contracts.md`. Change them only in their own small PR.
- Keep your devlog in `docs/<name>/DEVLOG.md`.

## Credits
- Imagery and height data by Vexcel Imaging US, Inc.
- Building data: CoStar Group; 3D tours: Matterport
- Sun position: NOAA solar calculation formulas
- Built with Rust (axum, tokio, serde), three.js and 3d-tiles-renderer, and Claude (Anthropic)

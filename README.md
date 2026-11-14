# `golden`
We allow you to find sunset positions for your photos!

Tell golden the shot you want ("sun setting between two towers, from a rooftop, late June"). 
It finds where to stand in the city, shows you the skyline and the sun's path from there, 
and tells you who to ask for access.
Design: [DESIGN.md](DESIGN.md).

(some screenshot here)

HackPrinceton Fall 2026 Entertainment track - CoStar "AI for the Built World"

## Run it

**Prerequisites:** Rust (stable) <!--TODO Gogi: Version-->, Node 20+ (once `web/` exists). Windows: run the scripts from PowerShell.

```powershell
.\build.ps1 # builds everything  <!-- TODO Gogi: what is "everything" -->
.\run.ps1 # starts the server
```
Then open http://localhost:8787.

Without the scripts (any OS), from the repo root:
```sh
cargo run --release -p server   # serves web/ at / and the API under /api
```
Open http://localhost:8787 (phone on the same Wi-Fi: set `GOLDEN_ADDR=0.0.0.0:8787` and open the laptop's IP).
Type a brief such as "sunset between two buildings from a rooftop in Princeton tonight" and press Find spots. The first
search in a new area downloads its heights (~10–30 s); after that it is cached. Keys go in `env/.env`; with no Gemini key
the agent runs as an offline mock, and opened as a file (no server) the page shows sample data.

## Test
```sh
cargo test --workspace
cd web && npm run build # once web/ exists
```

## Configuration
Set these as environment variables. **Never commit their values.**

| Variable | What for | If missing |
|---|---|---|
| `VEXCEL_API_KEY` | Height maps and aerial photos | Falls back to `VEXCEL_ENV_FILE`. With neither, areas already in the cache still work; new areas fail with a "no Vexcel key" error |
| `VEXCEL_ENV_FILE` | Path to a `.env` file with the keys (`VEXCEL_API_KEY=...`, `GEMINI_API_KEY=...`), instead of setting them directly. Keep it in `env/` (gitignored) | `env/.env` (so running from the repo root just works) |
| `VEXCEL_CACHE_DIR` | Where downloaded Vexcel height-map tiles are kept (gitignored). A 200 m area is ~300 MB, a 1 km area ~1 GB | `cache/` in the folder you run from |
| `LIDAR_CACHE_DIR` | Where USGS LiDAR downloads are kept (gitignored). ~20 MB per 150 m radius, ~100 MB per 1 km | `VEXCEL_CACHE_DIR`, else `cache/` |
| `GOOGLE_MAPS_API_KEY` | Photo-match agent's place search (Places API (New): coordinates, place photos). Same `env/.env` | Landmarks via Esri's geocoder, no place photos |
| `GEMINI_API_KEY` | Photo-match agent (planned, see DESIGN.md "Photo match"). Get one at aistudio.google.com/apikey; put it in `env/.env` | Agent off |
| `GEMINI_MODEL` | Gemini model ID for the agent; check it with the models list (DESIGN.md "Keys") | Agent off |
| `COSTAR_KIT_DIR` | Path to the CoStar data kit (kept out of git) | Building lookups use fixtures |
| TODO: Some AI key | The scouting agent | The agent runs on an offline mock |

PowerShell example (current window only):
```powershell
$env:VEXCEL_ENV_FILE = "$PWD\env\.env"
cargo run -p vexcel --example fetch_dsm -- 34.049 -118.251 60   # lat lon radius_m: checks the key works
```

## How it works
<!-- TODO by H20: 4–5 plain steps, e.g. -->
1. **Height map:** Vexcel's 7.5 cm surface model of every roof and tree, cached and downsampled to ~1 m.
2. **Sun:** NOAA formulas give the sun's direction and height for any time and place.
3. **Horizon:** from any spot, we trace toward the sunset to find exactly when buildings hide the sun.
4. **Search:** the whole area is scored against your brief, and the best spots go on the map.
5. **Agent:** turns the spots into a call sheet with times, backups, the view, and who to call (CoStar).

## Why not PhotoPills?
<!-- TODO by H20: short version of DESIGN.md "What we offer that PhotoPills doesn't" -->
PhotoPills tells you where the sun is. golden tells you where to stand, and shows you the shot, in a city full of buildings.

## Limits
<!-- TODO by H20: be honest; judges trust this -->
- Vexcel aerial photos and height maps: Los Angeles, Houston and Princeton. Elsewhere in the US, heights come
  from USGS LiDAR (older, coarser) and the map from Esri.
- CoStar buildings and contacts: Los Angeles and Houston only, so "who to call" is empty elsewhere.
- Height map date: LA flown 19–29 April 2026 (Vexcel collection `n34w118-us-ca-losangeles-2026`); newer buildings may be missing.
- What's computed vs estimated: <!-- TODO -->

## Repo map
| Folder | What | Owner |
|---|---|---|
| `crates/core` | Pure maths: coordinates, sun, horizon, scoring, search. No network, no files. | burvy |
| `crates/vexcel` | Height map and aerial photo download, cache, downsample | Gogi |
| `crates/lidar` | Backup height map outside Vexcel's area (USGS 3DEP LiDAR, public, no key), same `DsmGrid` | Gogi |
| `crates/costar` | CoStar kit loader + building lookup | Kinesiology <!-- TODO name --> |
| `crates/server` | HTTP API on :8787 and the agent | Gogi |
| `web/` | Map, brief box, spot card, 3D skyline | Frontend <!-- TODO name --> |
| `docs/` | Contracts, pitch, Devpost, devlogs (`docs/<name>/DEVLOG.md`) | everyone |
| `fixtures/` | One sample file per data shape in `docs/contracts.md` | everyone |
| `crates/contracts` | The shapes from `docs/contracts.md` as Rust structs. Pure, so `core` can use it; IO crates fill it | everyone (contract PRs only) |

## How we work
- Read [AGENTS.md](AGENTS.md) first (people and AI agents alike).
- One branch per piece of work: `<part>/<what>`, e.g. `core/horizon`. Open a PR to `main`, which gets reviewed before merge.
- Shared data shapes live in `docs/contracts.md`. Change them only in their own small PR.
- Keep your devlog in `docs/<name>/DEVLOG.md`.

## Credits
- Imagery and height data by Vexcel Imaging US, Inc.
- Building data: CoStar Group; 3D tours: Matterport
- Sun position: NOAA solar calculation formulas
- <!-- TODO: libraries (axum, MapLibre, deck.gl, three.js, ...) -->

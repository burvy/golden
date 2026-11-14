# `golden`
We allow you to find sunset positions for your photos!

Tell golden the shot you want ("sun setting between two towers, from a rooftop, late June"). It finds where to
stand in the city, shows you the skyline and the sun's path from there, and tells you who to ask for access.
Design and reasoning: [DESIGN.md](DESIGN.md).

> 📸 _Hero screenshot / GIF goes here (brief → spots → skyline view)._ <!-- TODO by H20 -->

HackPrinceton Fall 2026 · Entertainment track · CoStar "AI for the Built World" · demo city: Los Angeles

## Run it

**Prerequisites:** Rust (stable) <!-- TODO Ray: exact version -->, Node 20+ (once `web/` exists). Windows: run the scripts from PowerShell.

```powershell
.\build.ps1      # builds everything  <!-- TODO Ray: what it builds -->
.\run.ps1        # starts the server
```
Then open http://localhost:8787.

Without the scripts (any OS):
```sh
cargo run -p server        # TODO Ray: confirm
```

## Test
```sh
cargo test --workspace
cd web && npm run build    # once web/ exists
```

## Configuration
Set these as environment variables. **Never commit their values.**

| Variable | What for | If missing |
|---|---|---|
| `VEXCEL_API_KEY` (or `VEXCEL_ENV_FILE` = path to a file holding it) | Height maps and aerial photos | Vexcel features use cached data or fixtures <!-- TODO confirm --> |
| `COSTAR_KIT_DIR` | Path to the CoStar data kit (kept out of git) | Building lookups use fixtures |
| `ANTHROPIC_API_KEY` | The scouting agent (Claude) | The agent runs on an offline mock |

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
- Data only covers Los Angeles and Houston.
- Height map date: <!-- TODO --> (newer buildings may be missing).
- What's computed vs estimated: <!-- TODO -->

## Repo map
| Folder | What | Owner |
|---|---|---|
| `crates/core` | Pure maths: coordinates, sun, horizon, scoring, search. No network, no files. | burvy |
| `crates/vexcel` | Height map and aerial photo download, cache, downsample | Ray |
| `crates/costar` | CoStar kit loader + building lookup | Kinesiology <!-- TODO name --> |
| `crates/server` | HTTP API on :8787 and the agent | Ray |
| `web/` | Map, brief box, spot card, 3D skyline | Frontend <!-- TODO name --> |
| `docs/` | Contracts, pitch, Devpost, devlogs (`docs/<name>/DEVLOG.md`) | everyone |
| `fixtures/` | One sample file per data shape in `docs/contracts.md` | everyone |

## How we work
- Read [AGENTS.md](AGENTS.md) first (people and AI agents alike).
- One branch per piece of work: `<part>/<what>`, e.g. `core/horizon`. Open a PR to `main`, which gets reviewed before merge.
- Shared data shapes live in `docs/contracts.md`. Change them only in their own small PR.
- Keep your devlog in `docs/<name>/DEVLOG.md`.

## Credits
- Imagery and height data © Vexcel Imaging US, Inc.
- Building data: CoStar Group; 3D tours: Matterport
- Sun position: NOAA solar calculation formulas
- Agent: [Claude](https://www.anthropic.com/claude) (Anthropic API)
- <!-- TODO: libraries (axum, MapLibre, deck.gl, three.js, ...) -->

# Ray's devlog

## 11/14/26: localhost + sunset (branch `server/sun`)

**What exists now**
- Cargo workspace at the repo root with two crates:
  - `crates/core` (package `golden-core`, imported as `golden_core`). It isn't named `core` because that
    clashes with Rust's built-in `core` crate.
  - `crates/server` (package `server`): axum on `127.0.0.1:8787`.
- `golden_core::sun::sunset(lat, lon, date) -> Result<Sunset { time, azimuth_deg }, NoSunset>`
  - `date` is the local calendar day at the spot. `time` is UTC, so LA's 20:07 PDT sunset on June 21 comes
    back as `2026-06-22T03:07:41Z` (the next UTC day). That's correct, not a bug.
  - `NoSunset::PolarDay` / `PolarNight` near the poles (the sun never sets / never rises).
  - Also `sun_position(lat, lon, t) -> { azimuth_deg, elevation_deg }`, which we'll need for `SunPath`.
- `GET /api/sun?lat=..&lon=..&date=YYYY-MM-DD` returns `{"time": "...Z", "azimuth_deg": 299.34}`.
  400 for bad input, 422 when there's no sunset that day.
- `.\build.ps1` (add `-Test` to also run tests) and `.\run.ps1`.

**How the sun maths works (plain version)**
1. Turn the date into a Julian day, then into centuries since the year 2000.
2. From that, NOAA's formulas give the sun's *declination* (how far north/south of the equator it is)
   and the *equation of time* (how far sundials run ahead of or behind clocks, up to ~16 min).
3. Sunset hour angle: how many degrees past solar noon the sun is when it's 0.833° below a flat horizon
   (0.833° = atmospheric refraction + half the sun's disc). 1° of hour angle = 4 minutes.
4. Sunset (UTC minutes) = 720 − 4·(longitude − hour angle) − equation of time.
5. Do it twice: the second pass recomputes the sun's state at the first estimate (NOAA does the same).
6. Azimuth = the sun's direction at that instant.

**How it's tested against NOAA**
I downloaded NOAA's own calculator code (https://gml.noaa.gov/grad/solcalc/main.js), ran its functions
in Node (`docs/ray/noaa_ref.js`), and pasted the outputs into the test table in `crates/core/src/sun.rs`.
Cases: LA (both solstices, equinox), Houston, Greenwich, Sydney, and Tromsø for polar day/night.
Ours matches NOAA within 1 s and 0.01° of azimuth.

**Not done / open questions**
- The time is UTC only. Showing local time (PDT) needs a time zone. Either the web app converts it, or the
  API takes a `tz` param. This needs agreeing on in `docs/contracts.md`, which doesn't exist yet.
- This is sunset over a *flat* horizon. Buildings come later (horizon from the Vexcel DSM).

## 11/14/26: real API endpoints + fixture stubs (branch `server/endpoints`)

(Gogi here: this devlog lives under `docs/ray/` for now.)

**What the server answers now** (all shapes from `docs/contracts.md`, errors are `{"error": ...}`):
- `GET /api/sun`: the real `SunPath`: local times with the offset, sunrise/sunset, morning and evening golden
  hour, points every `step_min`. Replaces the old `{time, azimuth_deg}` answer. 422 on polar day/night.
- `GET /api/horizon`: height map around the point (Vexcel, else USGS LiDAR) -> burvy's `core::horizon`, plus
  `core::sunblock` when `date` is given.
- `POST /api/search`: height map of the area + 1500 m (so the skyline is seen from the area's edge) ->
  `core::search`. Roof spots get `building_id` = the nearest CoStar building within 50 m.
- `GET /api/buildings`, `/api/buildings/{id}`: from Kinesiology's `costar` crate. 503 if the server has no kit.
- `GET /api/scene` (+ `.bin`), `GET /api/photo`, `POST /api/agent`: still fixtures, header `x-golden-stub`.
- `GOLDEN_STUBS=1`: every endpoint answers with its fixture (frontend work with no keys, kit or network).
- `GOLDEN_ADDR` overrides `127.0.0.1:8787`. CORS is open so the web app can run on another local port.

**How sunrise and golden hour are found.** Sample the sun's apparent elevation every 5 min over the local day,
and bisect every crossing of a level to the second: -0.436° (top edge on the horizon, same as core's `sunset`
and `sunblock`), -4° and +6°. LA 21 June: sunset 20:07:41 (NOAA, to the second), sunrise 05:42:04 (NOAA's
sunrise formula says 05:42:12; 05:42:04 is when NOAA's own sun position crosses, and what core uses),
evening golden hour 19:31:27–20:25:54. The fixture's 19:19:40–20:24:20 was made up.

**Speed (live, downtown LA, 200 m roof search).** First call 157 s (downloads ~3.4 km of Vexcel), then 23 s
from the disk cache. That was too slow, so the server now keeps the last 3 grids in memory and reuses one
for any request inside it: the same search again takes 0.18 s, and a horizon inside the area 0.06 s.
Pre-fetch the demo area once after starting the server.

**For the frontend:** `web/index.html` still calls some old/mock shapes (`/api/spot/horizon`, a `sun`
answer that's an array, `brief` as a string in search). The real shapes are in `docs/contracts.md`.

## 11/14/26: the call sheet in the app (branch `web/call-sheet`)

(Gogi. My `server/endpoints` branch is dropped: Nube-Lube's commit on main covers it.)

The brief box already went to `POST /api/agent`, but the answer showed as a block of plain text. Now it's a
rendered call sheet right under the brief (web owner OK'd editing `web/index.html`):
- headline, date, summary; **timeline** with "the shot" highlighted;
- **spots**: primary/backup, arrive-by, when the sun meets the skyline, golden minutes, Vexcel photo, notes,
  "Show on map and view" (opens the spot card) and Directions;
- **who to call**: building, contact role/name/company, tap-to-call / email (the request pre-filled) / copy
  request. It also fetches `/api/buildings/{id}` for the address, other contacts with a phone or email, and the
  Matterport tour ("Look inside: 3D tour");
- **what the agent did** (`CallSheet.steps`) and the caveats. "Copy as text" kept.
- While waiting: the agent's pipeline with a pulse and a seconds counter (not live steps: that needs streaming).
- Everything from the server is HTML-escaped (CoStar and Gemini text).

**Agent fix:** in DTLA every roof spot came back with no CoStar building, so "who to call" was empty. Search
links a roof to a building only within 40 m of CoStar's point, which is the street address, often 50–100 m from
the roof on a big block. `Spot.building_id` stays strict; for the call sheet the agent now asks the nearest CoStar
building within 100 m and adds a caveat with the distance ("check it's the right building"). One request per
building. Tested live: 2 of 3 roofs got contacts (47 m, 69 m).

**Coverage update (same branch).** Vexcel now serves Princeton (`n40w74-us-nj-trenton-2026`, flown June 2026),
but the CoStar kit is still LA (~1,580 buildings) and Houston (~1,760) only: 0 in New Jersey. The agent now tells
the two cases apart: if CoStar has no building within 3 km of the area, the sheet gets one caveat ("CoStar covers
LA and Houston, not this area") and the buildings step says so, instead of "roof #n isn't matched" per spot.
Coverage wording updated in README (Limits), DESIGN (Where), and the vexcel/server comments.
Live: Princeton default brief, 15 s, Vexcel heights + obliques, 3 roof spots, no contacts (as expected);
downtown LA still gets its contacts.

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

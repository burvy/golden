# Web frontend devlog

## 2026-11-14 — offline hero and contract-shaped demo (`web/offline-hero-contracts`)

- Applied the supplied web patch without committing. The supplied ignore patch was redundant: `.DS_Store` is already ignored and none are tracked.
- Demo responses use contract wrappers and retain one synthetic search area so ranked spots and their detail cards agree. HTTP 4xx search errors are visible in the API status.
- Added flat hero and schematic map fallbacks, corrected photo updates after card rendering, and improved tab contrast.
- Cleared request timeout timers even when fetch rejects, preventing failed requests from delaying test process exit.
- Checked request parameters and Brief enums against `docs/contracts.md`; the handoff's uncertain query parameters, health endpoint and Vexcel tile route are documented there. Shared contracts and backend code were not changed.
- Validation: `npm run build` passed all 20 tests, including the committed fixture adapter smoke test, with no skips. `cargo test --workspace` passed 104 tests including doctests; 3 live Vexcel tests remain ignored. `git diff --check` passed.
- Browser scenarios reported in the supplied handoff were not rerun here. Live engine integration remains unverified; fixture smoke tests do not validate every contract field. Existing third-party service and CDN dependencies remain.
- Suggested human commit: `web: keep offline scouting usable and align demo data with contracts`.

## 2026-11-14 — cinematic scouting client (`web/map`)

Rebuilt `web/` as a cinematic location-scouting UI without touching Rust or contracts.

**What shipped**
- Full-screen Three.js homepage: live sun for the visitor’s coordinates, Open-Meteo weather, OSM building extrusion when Overpass answers, placeholder city otherwise.
- Dashboard: MapLibre (OpenFreeMap + Esri imagery + optional Vexcel tiles), city search, geolocation, shot brief, date, ranked spot cards, call-sheet copy.
- Spot preview: interpolated sun path, horizon silhouette, time slider, height-grid 3D (explicitly labelled as simulation, not photoreal mesh).
- API adapters in `web/js/adapters.js` that accept both `docs/contracts.md` payloads and the simpler shapes the existing HTML client used (`{brief,date,area}`, sun as an array, JPEG photo, flat building list).
- DEMO fallback around the **selected map centre** (no hardcoded Los Angeles bbox / `America/Los_Angeles` in the live flow). DEMO is labelled and never presented as a permitted site.

**How to run**
1. `cargo run -p server` (teammates’ API on `:8787`).
2. `cd web && python3 -m http.server 5173`
3. Open `http://localhost:5173`
4. `cd web && npm test` (adapter tests; `npm run build` is the same checks for CI).

**Not claimed**
- Photoreal 3D reconstruction
- Rooftop access rights
- Verified contacts when the buildings endpoint is down (those rows are DEMO)

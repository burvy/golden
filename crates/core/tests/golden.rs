//! Golden outputs: search must give byte-identical spots and heat to what `main` gave before the
//! metrics funnel (task 051). The JSON files in `tests/golden/` were captured on main de0498c.
//!
//! - `synthetic_city_matches_golden` runs everywhere (a made-up grid, no IO beyond the golden file).
//! - `real_heights_match_golden` uses the real LA / Princeton heights from the demo cache and is
//!   ignored by default. Run it with the cache (no key needed) in release:
//!   `VEXCEL_CACHE_DIR=Z:/3Projects/golden-cache cargo test --release -p golden-core --test golden -- --ignored`
//!
//! `GOLDEN_WRITE=1` rewrites the files instead of comparing (only do that on the old code).
//! Fields added after the capture (`spots[].scores`) are dropped before comparing.

use std::path::PathBuf;

use contracts::{Area, Brief, DsmGrid, DsmMeta, DsmSource, Event, Framing, Standing, NODATA};
use golden_core::search::{search, skyline_need_m};
use serde_json::{json, Value};

const LA: (f64, f64) = (34.0490, -118.2510);
const PRINCETON: (f64, f64) = (40.3487, -74.6593);
const FRAMINGS: [Framing; 3] = [Framing::Clear, Framing::Between, Framing::Behind];

struct Case {
    name: String,
    brief: Brief,
    utc_offset_s: i32,
}

fn brief(lat: f64, lon: f64, radius_m: f64, date: &str, event: Event, standing: Standing, framing: Framing) -> Brief {
    Brief {
        text: String::new(),
        area: Area { lat, lon, radius_m },
        date: date.into(),
        tz: contracts::search::DEFAULT_TZ.into(),
        event,
        standing,
        framing,
        max_spots: 10,
    }
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(format!("{name}.json"))
}

/// The parts of a search result the regression cares about, with later-added fields removed.
fn snapshot(grid: &DsmGrid, case: &Case) -> Value {
    let out = search(grid, &case.brief, case.utc_offset_s).expect("search");
    let mut v = json!({ "spots": out.spots, "heat": out.heat, "searched": out.searched });
    for spot in v["spots"].as_array_mut().unwrap() {
        spot.as_object_mut().unwrap().remove("scores");
    }
    v
}

/// Compare every case against its file (or write the files with GOLDEN_WRITE=1).
fn check(grid: &DsmGrid, cases: &[Case]) {
    let write = std::env::var("GOLDEN_WRITE").is_ok_and(|v| v == "1");
    let mut failed = Vec::new();
    let started = std::time::Instant::now();
    for case in cases {
        // Compared as text: serde_json's default float parsing can be 1 ulp off; its printing is exact.
        let got = serde_json::to_string_pretty(&snapshot(grid, case)).unwrap() + "\n";
        let path = golden_path(&case.name);
        if write {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n");
        if got != want {
            let at = got.lines().zip(want.lines()).position(|(g, w)| g != w).unwrap_or(0);
            let line = |s: &str| s.lines().nth(at).unwrap_or("<end>").to_string();
            eprintln!("MISMATCH {} line {}\n  want: {}\n  got:  {}", case.name, at + 1, line(&want), line(&got));
            failed.push(case.name.clone());
        }
    }
    eprintln!("{} searches in {:.2?}", cases.len(), started.elapsed());
    assert!(failed.is_empty(), "differs from golden: {failed:?}");
}

/// Every framing for each (place, date, event, standing) combination.
fn cases(prefix: &str, lat: f64, lon: f64, radius_m: f64, runs: &[(&str, i32, Event, Standing)]) -> Vec<Case> {
    let mut out = Vec::new();
    for &(date, utc_offset_s, event, standing) in runs {
        for framing in FRAMINGS {
            let name = format!("{prefix}-{date}-{event:?}-{standing:?}-{framing:?}").to_lowercase();
            out.push(Case { name, brief: brief(lat, lon, radius_m, date, event, standing, framing), utc_offset_s });
        }
    }
    out
}

// ---- synthetic -------------------------------------------------------------------------------

/// A 700 m square of 1 m cells around LA with a handful of towers, low blocks and a street canyon,
/// so every framing has something to find.
fn synthetic_city() -> DsmGrid {
    let n: u32 = 700;
    let half = f64::from(n) / 2.0;
    // (east_m, north_m, half_width_m, height_m)
    let boxes: [(f64, f64, f64, f32); 9] = [
        (-220.0, 40.0, 15.0, 120.0),
        (-200.0, -60.0, 12.0, 90.0),
        (-260.0, -10.0, 8.0, 60.0),
        (-120.0, 120.0, 25.0, 30.0),
        (40.0, 30.0, 20.0, 18.0),
        (60.0, -40.0, 10.0, 45.0),
        (-40.0, -90.0, 30.0, 12.0),
        (150.0, 150.0, 40.0, 25.0),
        (-300.0, -200.0, 20.0, 150.0),
    ];
    let mut data = Vec::with_capacity((n * n) as usize);
    for r in 0..n {
        for c in 0..n {
            let (e, no) = (f64::from(c) + 0.5 - half, half - f64::from(r) - 0.5);
            let h = boxes
                .iter()
                .filter(|(x, y, w, _)| (e - x).abs() < *w && (no - y).abs() < *w)
                .map(|b| b.3)
                .fold(0.0_f32, f32::max);
            data.push(h);
        }
    }
    let source = DsmSource { provider: "synthetic".into(), collection: None, captured_from: None, captured_to: None, native_res_m: 1.0 };
    let meta = DsmMeta { center_lat: LA.0, center_lon: LA.1, cell_m: 1.0, width: n, height: n, nodata: NODATA, source };
    DsmGrid { meta, data }
}

#[test]
fn synthetic_city_matches_golden() {
    let runs = [
        ("2026-06-21", -7 * 3600, Event::Sunset, Standing::Any),
        ("2026-11-14", -8 * 3600, Event::Sunset, Standing::Ground),
        ("2026-11-14", -8 * 3600, Event::Sunrise, Standing::Any),
    ];
    check(&synthetic_city(), &cases("synthetic", LA.0, LA.1, 150.0, &runs));
}

// ---- real heights (demo cache) ---------------------------------------------------------------

/// Same fetch as `examples/search_la.rs`: ~0.9 m tiles only, area + skyline.
fn real_grid(lat: f64, lon: f64, radius_m: f64) -> DsmGrid {
    let opts = vexcel::DsmOptions { near_radius_m: 1.0, ..vexcel::DsmOptions::default() };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(vexcel::get_dsm_with(lat, lon, skyline_need_m(radius_m), &opts)).expect("heights (set VEXCEL_CACHE_DIR to the demo cache)")
}

#[test]
#[ignore = "needs the demo height cache (VEXCEL_CACHE_DIR); run in release"]
fn real_heights_match_golden() {
    // The hero briefs: the reader presets for Downtown LA and Princeton, 300 m each.
    let la = [
        ("2026-11-14", -8 * 3600, Event::Sunset, Standing::Any),
        ("2026-06-21", -7 * 3600, Event::Sunset, Standing::Any),
        ("2026-11-14", -8 * 3600, Event::Sunset, Standing::Ground),
        ("2026-11-14", -8 * 3600, Event::Sunrise, Standing::Any),
    ];
    check(&real_grid(LA.0, LA.1, 300.0), &cases("la", LA.0, LA.1, 300.0, &la));
    let princeton = [
        ("2026-11-14", -5 * 3600, Event::Sunset, Standing::Any),
        ("2026-11-14", -5 * 3600, Event::Sunset, Standing::Ground),
    ];
    check(&real_grid(PRINCETON.0, PRINCETON.1, 300.0), &cases("princeton", PRINCETON.0, PRINCETON.1, 300.0, &princeton));
}

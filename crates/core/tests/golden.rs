//! Golden outputs: search must give byte-identical spots and heat to what `main` gave before the
//! metrics funnel (task 051). The JSON files in `tests/golden/` were captured on main de0498c.
//!
//! - `synthetic_city_matches_golden` runs everywhere (a made-up grid, no IO beyond the golden file).
//! - `real_heights_match_golden` uses the real LA / Princeton heights from the demo cache and is
//!   ignored by default. Run it with the cache (no key needed) in release:
//!   `VEXCEL_CACHE_DIR=Z:/3Projects/golden-cache cargo test --release -p golden-core --test golden -- --ignored`
//!
//! `GOLDEN_WRITE=1` rewrites the files instead of comparing (only do that on the old code).
//! Fields added after the capture (`spots[].scores`, the goal layers and stages) are dropped
//! before comparing. Every search here is also rebuilt from its goal layers with the browser's
//! rule (docs/contracts.md#goal-layers-and-stages), which must give back its heat and spots, and
//! `weight_edits_*` check that re-weighting in the browser picks what a new search would.

use std::path::PathBuf;

use contracts::{Area, Brief, DsmGrid, DsmMeta, DsmSource, Event, Framing, Goal, GoalLayer, HeatGrid, Standing, EARTH_R, NODATA};
use golden_core::search::{search, skyline_need_m, SearchOutput};
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
        area: Area { lat, lon, radius_m, name: None },
        date: date.into(),
        tz: contracts::search::DEFAULT_TZ.into(),
        event,
        standing,
        framing,
        max_spots: 10,
        goals: Vec::new(),
        lens_deg: contracts::DEFAULT_LENS_DEG,
    }
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(format!("{name}.json"))
}

/// The parts of a search result the regression cares about, with later-added fields removed.
/// Also checks that the goal layers rebuild it.
fn snapshot(grid: &DsmGrid, case: &Case) -> Value {
    let out = search(grid, &case.brief, case.utc_offset_s).expect("search");
    rebuilds(&out, &out.layers, case.brief.max_spots as usize, &case.name);
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

// ---- the browser's rule ----------------------------------------------------------------------

/// The browser's rule: an exact copy of `rebuild` in crates/contracts/tests/fixtures.rs (the
/// reference, docs/contracts.md#goal-layers-and-stages). Keep the two the same.
fn rebuild(searched: &HeatGrid, layers: &[GoalLayer], spacing_m: f64, n: usize) -> (Vec<Option<f64>>, Vec<usize>) {
    let cells = searched.values.len();
    let dropped = |i: usize| layers.iter().any(|l| l.gate.contains(&(i as u32)) || (l.must && l.heat.values[i].unwrap_or(0.0) <= 0.0));
    let mut ranked: Vec<(&GoalLayer, f64)> = layers.iter().filter(|l| l.weight > 0.0).map(|l| (l, l.weight)).collect();
    if ranked.is_empty() {
        ranked = layers.iter().map(|l| (l, 1.0 / layers.len() as f64)).collect();
    }
    let raw: Vec<Option<f64>> = (0..cells)
        .map(|i| {
            searched.values[i]?;
            Some(if dropped(i) { 0.0 } else { ranked.iter().map(|(l, w)| w * f64::from(l.heat.values[i].unwrap())).sum() })
        })
        .collect();
    let best = raw.iter().flatten().fold(0.0, |a: f64, &b| a.max(b));
    let heat = raw.iter().map(|v| v.map(|r| if best > 0.0 { r / best } else { 0.0 })).collect();
    let mut order: Vec<usize> = (0..cells).filter(|&i| raw[i].unwrap_or(0.0) > 0.0).collect();
    order.sort_by(|&a, &b| raw[b].unwrap().total_cmp(&raw[a].unwrap()).then(a.cmp(&b)));
    let h = &layers[0].heat;
    let mut picked: Vec<usize> = vec![];
    for i in order {
        if picked.len() == n {
            break;
        }
        if picked.iter().all(|&p| metres(cell_latlon(h, p), cell_latlon(h, i)) >= spacing_m) {
            picked.push(i);
        }
    }
    (heat, picked)
}

fn cell_latlon(h: &HeatGrid, i: usize) -> (f64, f64) {
    let (col, row) = ((i as u32 % h.width) as f64, (i as u32 / h.width) as f64);
    let east = (col + 0.5 - f64::from(h.width) / 2.0) * h.cell_m;
    let north = (f64::from(h.height) / 2.0 - row - 0.5) * h.cell_m;
    (h.center_lat + (north / EARTH_R).to_degrees(), h.center_lon + (east / (EARTH_R * h.center_lat.to_radians().cos())).to_degrees())
}

fn metres(a: (f64, f64), b: (f64, f64)) -> f64 {
    let north = (b.0 - a.0).to_radians() * EARTH_R;
    let east = (b.1 - a.1).to_radians() * EARTH_R * a.0.to_radians().cos();
    east.hypot(north)
}

/// The heat cell a spot stands for.
fn cell_of(h: &HeatGrid, lat: f64, lon: f64) -> usize {
    (0..h.values.len()).min_by(|&a, &b| metres(cell_latlon(h, a), (lat, lon)).total_cmp(&metres(cell_latlon(h, b), (lat, lon)))).unwrap()
}

/// `layers` (maybe re-weighted) rebuild `out`'s heat and spots, and the layers fit the output.
fn rebuilds(out: &SearchOutput, layers: &[GoalLayer], n: usize, name: &str) {
    for l in layers {
        assert_eq!((l.heat.width, l.heat.cell_m, l.heat.center_lat), (out.heat.width, out.heat.cell_m, out.heat.center_lat), "{name}");
        assert!(l.heat.values.iter().zip(&out.heat.values).all(|(a, b)| b.is_some() || a.is_none()), "{name}: no layer value off the heatmap");
        assert!(l.heat.values.iter().flatten().all(|v| (0.0..=1.0).contains(v)), "{name}");
    }
    assert!(out.stages.windows(2).all(|w| w[0].n >= w[1].n), "{name}: the funnel only narrows");
    assert_eq!(out.stages[1].n, out.searched, "{name}");
    let (heat, picked) = rebuild(&out.heat, layers, out.spacing_m, n);
    for (i, (mine, theirs)) in heat.iter().zip(&out.heat.values).enumerate() {
        assert_eq!(mine.is_some(), theirs.is_some(), "{name}: cell {i}");
        if let (Some(a), Some(b)) = (mine, theirs) {
            assert!((a - f64::from(*b)).abs() < 1e-3, "{name}: cell {i} heat {a} vs {b}");
        }
    }
    let theirs: Vec<usize> = out.spots.iter().map(|s| cell_of(&out.heat, s.lat, s.lon)).collect();
    assert_eq!(picked, theirs, "{name}: the browser picks the server's spots");
}

/// A brief with four goals: the sun (ranks), a roof preference (ranks), a cost function with its
/// own gate (sees the sun 5+ min) and a must (ground under 40 m, no weight).
fn goal_brief(b: &Brief, weights: [f64; 4]) -> Brief {
    let sun_name = golden_core::library::sun_goal_name(b.framing);
    let goals = json!([
        { "use": sun_name, "weight": weights[0] },
        { "def": { "name": "roof", "about": "On a roof", "score": { "f": "roof" } }, "weight": weights[1] },
        { "def": { "name": "sees_sun", "about": "Sees the sun 5+ minutes: in view long enough to shoot",
                   "must": { "op": "gt", "x": { "f": "sun_visible_min" }, "than": { "num": 5 } },
                   "score": { "op": "ramp", "x": { "f": "sun_visible_min" }, "lo": 5, "hi": 60 } }, "weight": weights[2] },
        { "def": { "name": "under_40m", "about": "Below 40 m", "score": { "op": "lt", "x": { "f": "ground_m" }, "than": { "num": 40 } } },
          "must": true, "weight": weights[3] }
    ]);
    Brief { goals: serde_json::from_value::<Vec<Goal>>(goals).unwrap(), ..b.clone() }
}

/// Re-weight the first search's layers in the browser; a new server search with those weights
/// must pick the same spots and draw the same heat.
fn weight_edits_match(grid: &DsmGrid, b: &Brief, utc_offset_s: i32, name: &str) {
    let first = search(grid, &goal_brief(b, [1.0, 0.5, 0.3, 0.0]), utc_offset_s).expect("search");
    assert_eq!(first.layers.len(), 4);
    assert_eq!(first.stages.iter().filter_map(|s| s.goal.as_deref()).collect::<Vec<_>>(), ["under_40m", "sees_sun"], "{name}: cheapest gate first");
    assert!(first.stages[3].n < first.stages[1].n, "{name}: the gates drop something");
    rebuilds(&first, &first.layers, b.max_spots as usize, name);
    // dropped by sees_sun's own gate (the second): under_40m ran first and keeps its score, the
    // sun and roof goals were never reached
    let sees = &first.layers[2];
    assert!(name != "synthetic" || !sees.gate.is_empty(), "the synthetic city has cells that gate drops");
    for &c in &sees.gate {
        let at = |l: usize| first.layers[l].heat.values[c as usize];
        assert_eq!((at(0), at(1), at(2)), (None, None, Some(0.0)), "{name}: cell {c}");
        assert!(at(3).is_some_and(|v| v > 0.0), "{name}: cell {c}");
    }
    for weights in [[0.2, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 1.0, 2.0], [0.0, 0.0, 1.0, 0.0]] {
        let mut layers = first.layers.clone();
        for (l, w) in layers.iter_mut().zip(weights) {
            l.weight = w;
        }
        let again = search(grid, &goal_brief(b, weights), utc_offset_s).expect("search");
        assert_eq!(
            again.layers.iter().map(|l| &l.heat).collect::<Vec<_>>(),
            first.layers.iter().map(|l| &l.heat).collect::<Vec<_>>(),
            "{name}: weights don't change the layers"
        );
        rebuilds(&again, &layers, b.max_spots as usize, &format!("{name} {weights:?}"));
    }
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

#[test]
fn weight_edits_on_the_synthetic_city() {
    let b = brief(LA.0, LA.1, 150.0, "2026-06-21", Event::Sunset, Standing::Any, Framing::Clear);
    weight_edits_match(&synthetic_city(), &b, -7 * 3600, "synthetic");
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

#[test]
#[ignore = "needs the demo height cache (VEXCEL_CACHE_DIR); run in release"]
fn weight_edits_on_real_heights() {
    let la = brief(LA.0, LA.1, 300.0, "2026-11-14", Event::Sunset, Standing::Any, Framing::Between);
    weight_edits_match(&real_grid(LA.0, LA.1, 300.0), &la, -8 * 3600, "la");
    let pr = brief(PRINCETON.0, PRINCETON.1, 300.0, "2026-11-14", Event::Sunset, Standing::Any, Framing::Clear);
    weight_edits_match(&real_grid(PRINCETON.0, PRINCETON.1, 300.0), &pr, -5 * 3600, "princeton");
}

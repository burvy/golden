//! Test helpers: synthetic height grids and an `AppState` that never touches the network, a key or the
//! demo cache. Any height-map request the inserted grid doesn't cover fails instead of downloading.

use std::path::PathBuf;
use std::sync::Arc;

use contracts::{DsmGrid, DsmMeta, DsmSource, NODATA};

use crate::state::{ApiErr, AppState, Grids};

/// Downtown LA: the grids below are centred here (tz America/Los_Angeles).
pub const LAT: f64 = 34.05;
pub const LON: f64 = -118.25;
/// Cell size and half-width of the test grids: 2.1 km covers a 500 m search plus its 1.5 km skyline.
const CELL_M: f64 = 5.0;
const HALF_M: f64 = 2100.0;

/// A flat grid at height 0, with `blocks` (west, east, south, north edges in metres from the centre, height in
/// metres) raised on it.
pub fn grid(blocks: &[(f64, f64, f64, f64, f32)]) -> DsmGrid {
    let n = (2.0 * HALF_M / CELL_M) as u32;
    let mut data = vec![0.0f32; (n * n) as usize];
    for r in 0..n {
        for c in 0..n {
            let east = (f64::from(c) + 0.5) * CELL_M - HALF_M;
            let north = HALF_M - (f64::from(r) + 0.5) * CELL_M;
            for &(w, e, s, nn, h) in blocks {
                if (w..e).contains(&east) && (s..nn).contains(&north) {
                    data[(r * n + c) as usize] = h;
                }
            }
        }
    }
    let source = DsmSource { provider: "synthetic".into(), collection: None, captured_from: None, captured_to: None, native_res_m: CELL_M };
    DsmGrid { meta: DsmMeta { center_lat: LAT, center_lon: LON, cell_m: CELL_M, width: n, height: n, nodata: NODATA, source }, data }
}

/// An 80 m wall 200-250 m west of the centre, running 300 m north and south: it hides the June sunset
/// (azimuth ~299 deg) from the centre at about 21 deg up. Plus a few towers so searches find roofs.
pub fn city() -> DsmGrid {
    grid(&[
        (-250.0, -200.0, -300.0, 300.0, 80.0),
        (100.0, 140.0, 100.0, 140.0, 45.0),
        (160.0, 200.0, 100.0, 140.0, 50.0),
        (-60.0, -20.0, -180.0, -140.0, 30.0),
    ])
}

/// State holding `grid`, no CoStar kit, scenes written under a fresh temp path (only created if a scene is
/// built; that test removes it). Height maps it doesn't cover fail.
pub fn state(grid: DsmGrid) -> AppState {
    let grids = Grids::with_fetch(Arc::new(|lat, lon, r| {
        Box::pin(async move { Err(ApiErr::upstream(format!("offline test: no grid for {lat:.4},{lon:.4} r {r:.0} m"))) })
    }));
    grids.insert(grid);
    AppState { grids, scenes: Default::default(), kit: None, http: reqwest::Client::new(), cache_root: temp_path("state") }
}

/// A new empty directory under the system temp dir.
pub fn temp_dir(tag: &str) -> PathBuf {
    let d = temp_path(tag);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A fresh path under the system temp dir, not created.
pub fn temp_path(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!("golden-test-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

//! vexcel: height maps (DSM) from Vexcel, cached on disk, downsampled to a ground-metre `DsmGrid`;
//! and oblique aerial photos (`get_oblique`), see `oblique.rs`.
//!
//! ```no_run
//! # async fn demo() -> Result<(), vexcel::Error> {
//! let grid = vexcel::get_dsm(34.0490, -118.2510, 300.0).await?;
//! println!("{} x {} cells of {} m", grid.meta.width, grid.meta.height, grid.meta.cell_m);
//! # Ok(()) }
//! ```
//!
//! How it fetches (numbers measured on the live API, see docs/Nube-Lube/DEVLOG.md):
//! - Near the point (`near_radius_m`, default 150 m): level 0 tiles, 1024 px of ~6 cm (~63 m per tile, ~3 MB).
//! - Further out: level 1 tiles, 4608 px of ~12 cm (~570 m per tile). At 1 m output cells the far
//!   field loses nothing, and it costs 4x fewer bytes than level 0 for the same area.
//! - Every tile is cached under `cache/vexcel/dsm/`, so a second call for the same area makes no requests.

pub mod auth;
pub mod cache;
pub mod client;
pub mod geotiff;
pub mod grid;
pub mod oblique;
pub mod resample;
pub mod tiles;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use contracts::{DsmGrid, DsmMeta, DsmSource, NODATA};
pub use oblique::{get_oblique, get_oblique_with, Facing, Oblique, ObliqueOptions};

use auth::ApiKey;
use resample::GridSpec;
use tiles::TileId;

/// Biggest radius we will fetch. Horizons need 1-3 km; anything more is "fetching a city".
pub const MAX_RADIUS_M: f64 = 3000.0;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Vexcel key: set VEXCEL_API_KEY, or VEXCEL_ENV_FILE to a .env file containing it")]
    NoKey,
    #[error("radius {0} m is over the {MAX_RADIUS_M} m limit")]
    TooLarge(f64),
    #[error("Vexcel returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("tiff: {0}")]
    Tiff(#[from] tiff::TiffError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    DsmFile(#[from] contracts::DsmFileError),
    #[error("bad data: {0}")]
    BadData(String),
}

#[derive(Debug, Clone)]
pub struct DsmOptions {
    /// Output cell size in ground metres.
    pub cell_m: f64,
    /// Within this radius use the finest (~6 cm) data; beyond it, ~12 cm.
    pub near_radius_m: f64,
    /// Parallel requests to Vexcel.
    pub max_concurrency: usize,
    pub cache_root: PathBuf,
}

impl Default for DsmOptions {
    fn default() -> Self {
        DsmOptions { cell_m: 1.0, near_radius_m: 150.0, max_concurrency: 4, cache_root: cache::default_root() }
    }
}

/// Height map of a square of side `2 * radius_m` around a point, at 1 m cells.
pub async fn get_dsm(lat: f64, lon: f64, radius_m: f64) -> Result<DsmGrid, Error> {
    get_dsm_with(lat, lon, radius_m, &DsmOptions::default()).await
}

pub async fn get_dsm_with(lat: f64, lon: f64, radius_m: f64, opts: &DsmOptions) -> Result<DsmGrid, Error> {
    if !(radius_m > 0.0 && radius_m <= MAX_RADIUS_M) {
        return Err(Error::TooLarge(radius_m));
    }
    let tiles = plan_tiles(lat, lon, radius_m, opts.near_radius_m);
    ensure_cached(&tiles, opts).await?;
    let spec = GridSpec::around(lat, lon, radius_m, opts.cell_m);
    let root = opts.cache_root.clone();
    tokio::task::spawn_blocking(move || build_grid(spec, &tiles, &root))
        .await
        .map_err(|e| Error::BadData(format!("grid build panicked: {e}")))?
}

/// Which tiles to fetch: level 0 near the point, level 1 for the rest (if any).
pub fn plan_tiles(lat: f64, lon: f64, radius_m: f64, near_radius_m: f64) -> Vec<TileId> {
    // half a metre of margin so edge cells get every source pixel
    let near = radius_m.min(near_radius_m) + 0.5;
    let mut out = tiles::tiles_covering(0, tiles::ground_box(lat, lon, near));
    if radius_m > near_radius_m {
        out.extend(tiles::tiles_covering(1, tiles::ground_box(lat, lon, radius_m + 0.5)));
    }
    out
}

/// Download every tile not already on disk, a few at a time.
pub async fn ensure_cached(tiles: &[TileId], opts: &DsmOptions) -> Result<(), Error> {
    let missing: Vec<TileId> =
        tiles.iter().copied().filter(|t| !cache::tile_path(&opts.cache_root, *t).exists()).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let key = ApiKey::from_env()?;
    let http = client::http()?;
    let gate = Arc::new(tokio::sync::Semaphore::new(opts.max_concurrency.max(1)));
    let mut jobs = tokio::task::JoinSet::new();
    for t in missing {
        let (http, key, gate) = (http.clone(), key.clone(), gate.clone());
        let path = cache::tile_path(&opts.cache_root, t);
        jobs.spawn(async move {
            let _permit = gate.acquire_owned().await.expect("semaphore closed");
            let bytes = client::fetch_tile(&http, &key, t).await?;
            cache::write(&path, &bytes).await?;
            Ok::<(), Error>(())
        });
    }
    while let Some(done) = jobs.join_next().await {
        done.map_err(|e| Error::BadData(format!("download task failed: {e}")))??;
    }
    Ok(())
}

/// Decode cached tiles one at a time (a level 1 tile is ~85 MB decoded) into the output grid.
fn build_grid(spec: GridSpec, tiles: &[TileId], cache_root: &Path) -> Result<DsmGrid, Error> {
    let mut fine = spec.empty();
    let mut coarse = spec.empty();
    let mut source = DsmSource {
        provider: "vexcel".into(),
        collection: None,
        captured_from: None,
        captured_to: None,
        native_res_m: f64::INFINITY,
    };
    let cos_lat = spec.center_lat.to_radians().cos();
    for t in tiles {
        let bytes = std::fs::read(cache::tile_path(cache_root, *t))?;
        let r = geotiff::decode(&bytes)?;
        source.native_res_m = source.native_res_m.min(r.scale_x * cos_lat);
        if source.collection.is_none() {
            source.collection = r.info.collection.clone();
            source.captured_from = r.info.captured_from.clone();
            source.captured_to = r.info.captured_to.clone();
        }
        let target = if t.level == 0 { &mut fine } else { &mut coarse };
        resample::accumulate_max(&spec, target, &r);
    }
    Ok(DsmGrid {
        meta: DsmMeta {
            center_lat: spec.center_lat,
            center_lon: spec.center_lon,
            cell_m: spec.cell_m,
            width: spec.width,
            height: spec.height,
            nodata: NODATA,
            source,
        },
        data: resample::overlay(&fine, &coarse),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_radius_uses_only_fine_tiles() {
        let t = plan_tiles(34.049, -118.251, 100.0, 150.0);
        assert!(!t.is_empty() && t.iter().all(|t| t.level == 0));
    }

    #[test]
    fn large_radius_adds_coarse_tiles() {
        let t = plan_tiles(34.049, -118.251, 1000.0, 150.0);
        let coarse = t.iter().filter(|t| t.level == 1).count();
        assert!(coarse >= 4 && coarse <= 25, "{coarse} coarse tiles");
        assert!(t.iter().filter(|t| t.level == 0).count() <= 49);
    }

    #[tokio::test]
    async fn refuses_city_sized_requests() {
        assert!(matches!(get_dsm(34.0, -118.0, 50_000.0).await, Err(Error::TooLarge(_))));
        assert!(matches!(get_dsm(34.0, -118.0, -1.0).await, Err(Error::TooLarge(_))));
    }

    #[tokio::test]
    async fn builds_from_cache_without_network() {
        // Put a synthetic GeoTIFF where the cache expects the tile, then ask for that area.
        let root = std::env::temp_dir().join(format!("vexcel_lib_test_{}", std::process::id()));
        let (lat, lon) = (34.049, -118.251);
        let opts = DsmOptions { cache_root: root.clone(), ..DsmOptions::default() };
        let tiles = plan_tiles(lat, lon, 20.0, opts.near_radius_m);
        for t in &tiles {
            let (w, s, e, n) = tiles::tile_bounds(*t);
            let px = 64u32;
            let scale = (e - w) / f64::from(px);
            assert!((n - s - (e - w)).abs() < 1e-6);
            let bytes = geotiff::tests::make_tiff(px, px, w, n, scale, &vec![42.0; (px * px) as usize]);
            cache::write(&cache::tile_path(&root, *t), &bytes).await.unwrap();
        }
        let g = get_dsm_with(lat, lon, 20.0, &opts).await.unwrap();
        std::fs::remove_dir_all(&root).ok();
        assert_eq!((g.meta.width, g.meta.height), (40, 40));
        assert!(g.data.iter().all(|&v| v == 42.0));
        assert_eq!(g.meta.source.collection.as_deref(), Some("test-coll"));
    }

    /// Live: hits Vexcel. Run with `cargo test -p vexcel -- --ignored live`.
    #[tokio::test]
    #[ignore]
    async fn live_downtown_la() {
        let g = get_dsm(34.0490, -118.2510, 60.0).await.expect("fetch");
        let valid: Vec<f32> = g.data.iter().copied().filter(|&v| v != NODATA).collect();
        assert!(valid.len() as f64 > 0.95 * g.data.len() as f64, "too much nodata");
        let max = valid.iter().cloned().fold(f32::MIN, f32::max);
        let min = valid.iter().cloned().fold(f32::MAX, f32::min);
        // downtown LA ground is ~80-90 m above sea level, roofs here go to ~140 m
        assert!((60.0..120.0).contains(&min) && max > 120.0, "min {min} max {max}");
        assert!(g.meta.source.native_res_m < 0.08, "native {} m", g.meta.source.native_res_m);
    }
}

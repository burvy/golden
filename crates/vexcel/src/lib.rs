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
//! - Further out: `FAR_LEVEL` tiles, 2048 px of ~0.9 m (~1.9 km per tile at 40° N), fetched with Vexcel's
//!   `zoom=17`. At 1 m output cells that's all a skyline needs, and a 2 km box is ~16 MB instead of the
//!   ~700 MB the old 12 cm level 1 tiles cost.
//! - Every tile is cached under `cache/vexcel/dsm/`, so a second call for the same area makes no requests.

pub mod auth;
pub mod cache;
pub mod client;
pub mod mesh;
pub mod geotiff;
pub mod grid;
pub mod oblique;
pub mod ortho;
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
    /// Within this radius use the finest (~6 cm) data; beyond it, ~0.9 m.
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
    let planned = plan_tiles(lat, lon, radius_m, opts.near_radius_m);
    let tiles = ensure_cached(&planned, opts).await?;
    let stood_in = tiles != planned;
    let spec = GridSpec::around(lat, lon, radius_m, opts.cell_m);
    let root = opts.cache_root.clone();
    let mut grid = tokio::task::spawn_blocking(move || build_grid(spec, &tiles, &root))
        .await
        .map_err(|e| Error::BadData(format!("grid build panicked: {e}")))??;
    if stood_in {
        // Some of the near field is ~0.9 m data, so don't claim the 6 cm of whichever fine tiles were cached.
        grid.meta.source.native_res_m = tiles::scale(tiles::FAR_LEVEL) * lat.to_radians().cos();
    }
    Ok(grid)
}

/// Which tiles to fetch: level 0 near the point, `FAR_LEVEL` for the rest (if any).
pub fn plan_tiles(lat: f64, lon: f64, radius_m: f64, near_radius_m: f64) -> Vec<TileId> {
    // half a metre of margin so edge cells get every source pixel
    let near = radius_m.min(near_radius_m) + 0.5;
    let mut out = tiles::tiles_covering(0, tiles::ground_box(lat, lon, near));
    if radius_m > near_radius_m {
        out.extend(tiles::tiles_covering(tiles::FAR_LEVEL, tiles::ground_box(lat, lon, radius_m + 0.5)));
    }
    out
}

/// Download every tile not already on disk, a few at a time, and return the tiles to build the grid from.
///
/// That's `tiles` itself, unless the fine (level 0) tiles can't be fetched: with no key, or a key Vexcel
/// refuses (401/403, e.g. once it has expired), each missing fine tile is replaced by the cached `FAR_LEVEL`
/// tiles under it (see `cached_only`). `NoKey` (or Vexcel's refusal) only if those aren't cached either.
pub async fn ensure_cached(tiles: &[TileId], opts: &DsmOptions) -> Result<Vec<TileId>, Error> {
    let on_disk = |t: TileId| cache::tile_path(&opts.cache_root, t).exists();
    let missing: Vec<TileId> = tiles.iter().copied().filter(|t| !on_disk(*t)).collect();
    if missing.is_empty() {
        return Ok(tiles.to_vec());
    }
    let refused = match ApiKey::from_env() {
        Ok(key) => match download(&missing, key, opts).await {
            Ok(()) => return Ok(tiles.to_vec()),
            Err(e @ Error::Api { status: 401 | 403, .. }) => e,
            Err(e) => return Err(e),
        },
        Err(Error::NoKey) => Error::NoKey,
        Err(e) => return Err(e),
    };
    match cached_only(tiles, on_disk) {
        Some(built) => {
            let fine = missing.iter().filter(|t| t.level == 0).count();
            eprintln!(
                "vexcel: {} ({fine} of {} fine tiles not cached): building from the cached ~0.9 m tiles there",
                if matches!(refused, Error::NoKey) { "no key" } else { "key refused" },
                tiles.iter().filter(|t| t.level == 0).count(),
            );
            Ok(built)
        }
        None => Err(refused),
    }
}

/// The tiles to build from using only the cache: every planned tile, except that a missing level 0 tile is
/// replaced by the `FAR_LEVEL` tiles that cover it (`build_grid` lays fine data over coarse, so the cached
/// fine tiles still win where they exist). `None` if a needed tile still isn't cached.
pub fn cached_only(tiles: &[TileId], cached: impl Fn(TileId) -> bool) -> Option<Vec<TileId>> {
    let mut out: Vec<TileId> = Vec::with_capacity(tiles.len());
    let mut add = |t: TileId| {
        if !out.contains(&t) {
            out.push(t);
        }
    };
    for &t in tiles {
        if cached(t) {
            add(t);
        } else if t.level == 0 {
            for c in tiles::tiles_covering(tiles::FAR_LEVEL, tiles::tile_bounds(t)) {
                if !cached(c) {
                    return None;
                }
                add(c);
            }
        } else {
            return None;
        }
    }
    Some(out)
}

async fn download(missing: &[TileId], key: ApiKey, opts: &DsmOptions) -> Result<(), Error> {
    let http = client::http()?;
    let gate = Arc::new(tokio::sync::Semaphore::new(opts.max_concurrency.max(1)));
    let mut jobs = tokio::task::JoinSet::new();
    for &t in missing {
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
        let coarse = t.iter().filter(|t| t.level == tiles::FAR_LEVEL).count();
        assert!((1..=4).contains(&coarse), "{coarse} coarse tiles");
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

    /// A flat synthetic GeoTIFF of `value` filling tile `t`, written where the cache expects it.
    async fn put_tile(root: &Path, t: TileId, value: f32) {
        let (w, _, e, n) = tiles::tile_bounds(t);
        let px = 64u32;
        let bytes = geotiff::tests::make_tiff(px, px, w, n, (e - w) / f64::from(px), &vec![value; (px * px) as usize]);
        cache::write(&cache::tile_path(root, t), &bytes).await.unwrap();
    }

    #[test]
    fn cached_only_swaps_missing_fine_tiles_for_the_coarse_ones_under_them() {
        let planned = plan_tiles(34.049, -118.251, 100.0, 150.0);
        assert!(planned.len() > 1);
        let kept = planned[0];
        let coarse = tiles::tiles_covering(tiles::FAR_LEVEL, tiles::ground_box(34.049, -118.251, 101.0));
        let cached = |t: TileId| t == kept || coarse.contains(&t);
        let got = cached_only(&planned, cached).expect("coarse covers it");
        assert!(got.contains(&kept), "a cached fine tile is still used");
        assert!(got.iter().all(|t| t.level == tiles::FAR_LEVEL || *t == kept));
        assert!(coarse.iter().all(|c| got.iter().filter(|t| *t == c).count() == 1), "each coarse tile once");
    }

    #[test]
    fn cached_only_gives_up_when_coarse_is_missing_too() {
        let planned = plan_tiles(34.049, -118.251, 100.0, 150.0);
        let kept = planned[0];
        assert_eq!(cached_only(&planned, |t| t == kept), None);
        assert_eq!(cached_only(&planned, |t| t.level == 0), Some(planned.clone()), "all fine cached: unchanged");
        // a missing far tile has no stand-in
        let big = plan_tiles(34.049, -118.251, 1000.0, 150.0);
        assert_eq!(cached_only(&big, |t| t.level == 0), None);
    }

    #[tokio::test]
    async fn no_key_builds_from_cached_coarse_tiles() {
        if ApiKey::from_env().is_ok() {
            eprintln!("skipped: a Vexcel key is set, so missing tiles would be downloaded");
            return;
        }
        let root = std::env::temp_dir().join(format!("vexcel_nokey_test_{}", std::process::id()));
        let (lat, lon) = (34.049, -118.251);
        // 40 m cells: the synthetic coarse tiles are 64 px (~31 m pixels), and each pixel lands in one cell only
        // (real far tiles are ~1 m pixels, so 1 m cells fill).
        let opts = DsmOptions { cache_root: root.clone(), cell_m: 40.0, ..DsmOptions::default() };
        let planned = plan_tiles(lat, lon, 100.0, opts.near_radius_m);
        // No coarse tiles yet: nothing to stand in, so NoKey as before.
        put_tile(&root, planned[0], 42.0).await;
        assert!(matches!(get_dsm_with(lat, lon, 100.0, &opts).await, Err(Error::NoKey)));
        // Coarse coverage cached: the grid builds, fine data where it's cached, coarse everywhere else.
        for c in tiles::tiles_covering(tiles::FAR_LEVEL, tiles::ground_box(lat, lon, 101.0)) {
            put_tile(&root, c, 10.0).await;
        }
        let g = get_dsm_with(lat, lon, 100.0, &opts).await.unwrap();
        std::fs::remove_dir_all(&root).ok();
        assert!(g.data.iter().all(|&v| v == 42.0 || v == 10.0), "no holes");
        assert!(g.data.iter().any(|&v| v == 42.0) && g.data.iter().any(|&v| v == 10.0));
        assert_eq!(g.meta.source.provider, "vexcel");
        assert!(g.meta.source.native_res_m > 0.5, "says coarse, got {}", g.meta.source.native_res_m);
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

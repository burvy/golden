//! lidar: the backup height map for places Vexcel doesn't cover (our Vexcel key: only greater LA and Houston).
//!
//! Source: USGS 3DEP LiDAR point clouds, public on AWS in Entwine (EPT) format, no key. Same output as
//! `vexcel::get_dsm`: a ground-metre `DsmGrid` keeping the highest return per cell, so `core` and the
//! server don't care where the heights came from.
//!
//! ```no_run
//! # async fn demo() -> Result<(), lidar::Error> {
//! let grid = lidar::get_dsm(40.3487, -74.6593, 300.0).await?; // Nassau Hall, Princeton
//! println!("{} x {} cells, from {:?}", grid.meta.width, grid.meta.height, grid.meta.source.collection);
//! # Ok(()) }
//! ```
//!
//! How it fetches (measured at Princeton, project USGS_LPC_DE_DelawareValley_HD_2015_LAS_2017):
//! - The index of all projects (~9 MB GeoJSON) once; pick the newest survey covering the point.
//! - Within `near_radius_m` (150 m) every octree level, down to ~0.5 m point spacing: a 300 m square is
//!   ~36 tiles, ~20 MB, ~630k points.
//! - Further out only levels down to ~1 m spacing (`far_spacing_m`), which is plenty for 1 m cells.
//! - Everything is cached under `cache/lidar/` (`LIDAR_CACHE_DIR`), so the second call makes no requests.
//! Heights are metres above sea level (NAVD88). Vexcel's are on its own datum; only differences within one
//! grid matter for a horizon, so never mix the two in one grid.

pub mod ept;
pub mod fetch;
pub mod grid;
pub mod index;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub use contracts::{DsmGrid, DsmMeta, DsmSource, NODATA};

use ept::{Cube, Key};
use grid::Spec;
use index::Project;

/// Public index of every USGS LiDAR project in EPT form.
pub const INDEX_URL: &str = "https://usgs.entwine.io/boundaries/resources.geojson";
/// Same limit as Vexcel: horizons need 1-3 km; more is "fetching a city".
pub const MAX_RADIUS_M: f64 = 3000.0;
/// Point-cloud tiles one height map may use (a 3 km square at the default spacing is well under this). Stops
/// a dense survey from pulling hundreds of MB into memory.
pub const MAX_NODES: usize = 2500;
/// Hierarchy pages fetched per walk (each level of the octree is one round of pages).
const MAX_PAGE_ROUNDS: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("radius {0} m is over the {MAX_RADIUS_M} m limit")]
    TooLarge(f64),
    #[error("no USGS LiDAR covers {0}, {1}")]
    NoCoverage(f64, f64),
    #[error("HTTP {status} from {url}")]
    Api { status: u16, url: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("laz: {0}")]
    Las(#[from] las::Error),
    #[error("bad data: {0}")]
    BadData(String),
    #[error("not cached, and downloads are off (GOLDEN_OFFLINE=1): {0}")]
    Offline(String),
}

#[derive(Debug, Clone)]
pub struct DsmOptions {
    /// Output cell size in ground metres.
    pub cell_m: f64,
    /// Within this radius use every level of the point cloud; beyond it, stop at `far_spacing_m`.
    pub near_radius_m: f64,
    /// Coarsest point spacing (ground metres) to accept in the far field.
    pub far_spacing_m: f64,
    /// Parallel downloads.
    pub max_concurrency: usize,
    pub cache_root: PathBuf,
}

impl Default for DsmOptions {
    fn default() -> Self {
        DsmOptions {
            cell_m: 1.0,
            near_radius_m: 150.0,
            far_spacing_m: 1.0,
            max_concurrency: 4,
            cache_root: fetch::default_root(),
        }
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
    let http = fetch::http()?;
    let project = find_project(&http, opts, lat, lon).await?;
    let base = project.base_url().to_string();
    let dir = opts.cache_root.join("lidar").join(&project.name);

    let ept: EptJson = serde_json::from_slice(&fetch::cached(&http, &format!("{base}ept.json"), &dir.join("ept.json")).await?)?;
    if ept.srs.horizontal.as_deref() != Some("3857") {
        return Err(Error::BadData(format!("{} is not in EPSG:3857", project.name)));
    }
    let cube = Cube::from_bounds(&ept.bounds).ok_or_else(|| Error::BadData("ept.json bounds".into()))?;

    let spec = Spec::around(lat, lon, radius_m, opts.cell_m);
    let near_spec = Spec::around(lat, lon, radius_m.min(opts.near_radius_m), opts.cell_m);
    let (near, far) = (near_spec.merc_box(1.0), spec.merc_box(1.0));
    let cos = lat.to_radians().cos();
    // deepest level whose spacing is still coarser than far_spacing_m: one more level reaches it
    let far_depth = (0..32).find(|&d| cube.spacing(d, ept.span) * cos <= opts.far_spacing_m).unwrap_or(31);

    // walk the hierarchy, fetching pages as the walk needs them
    let mut hier: HashMap<String, i64> = HashMap::new();
    let mut pages = vec![Key::ROOT];
    let mut rounds = 0;
    let nodes = loop {
        rounds += 1;
        if rounds > MAX_PAGE_ROUNDS {
            return Err(Error::BadData(format!("{}: hierarchy deeper than {MAX_PAGE_ROUNDS} levels", project.name)));
        }
        for k in &pages {
            let name = k.name();
            let url = format!("{base}ept-hierarchy/{name}.json");
            let page: HashMap<String, i64> =
                serde_json::from_slice(&fetch::cached(&http, &url, &dir.join("hierarchy").join(format!("{name}.json"))).await?)?;
            hier.extend(page);
        }
        let plan = ept::plan(&hier, &cube, &near, &far, far_depth);
        if plan.pages.is_empty() {
            break plan.nodes;
        }
        pages = plan.pages;
    };

    if nodes.len() > MAX_NODES {
        return Err(Error::BadData(format!("{} point-cloud tiles needed, over the {MAX_NODES} limit: ask for a smaller radius", nodes.len())));
    }
    let paths = download_nodes(&http, &base, &dir, &nodes, opts.max_concurrency).await?;
    let deepest = nodes.iter().map(|k| k.d).max().unwrap_or(0);
    let source = DsmSource {
        provider: "usgs-3dep".into(),
        collection: Some(project.name.clone()),
        captured_from: project.year().map(|y| y.to_string()),
        captured_to: None,
        native_res_m: cube.spacing(deepest, ept.span) * cos,
    };
    tokio::task::spawn_blocking(move || build_grid(spec, &paths, source))
        .await
        .map_err(|e| Error::BadData(format!("grid build panicked: {e}")))?
}

#[derive(serde::Deserialize)]
struct EptJson {
    bounds: Vec<f64>,
    span: f64,
    srs: Srs,
}
#[derive(serde::Deserialize)]
struct Srs {
    horizontal: Option<String>,
}

/// The newest project whose outline contains the point.
async fn find_project(http: &reqwest::Client, opts: &DsmOptions, lat: f64, lon: f64) -> Result<Project, Error> {
    let path = opts.cache_root.join("lidar").join("resources.geojson");
    let bytes = fetch::cached(http, INDEX_URL, &path).await?;
    let text = String::from_utf8_lossy(&bytes);
    index::covering(&text, lat, lon)?.into_iter().next().ok_or(Error::NoCoverage(lat, lon))
}

async fn download_nodes(
    http: &reqwest::Client,
    base: &str,
    dir: &std::path::Path,
    nodes: &[Key],
    max_concurrency: usize,
) -> Result<Vec<PathBuf>, Error> {
    let gate = Arc::new(tokio::sync::Semaphore::new(max_concurrency.max(1)));
    let mut jobs = tokio::task::JoinSet::new();
    let mut paths = Vec::with_capacity(nodes.len());
    for k in nodes {
        let path = dir.join("data").join(format!("{}.laz", k.name()));
        paths.push(path.clone());
        if path.exists() {
            continue;
        }
        let (http, gate, url) = (http.clone(), gate.clone(), format!("{base}ept-data/{}.laz", k.name()));
        jobs.spawn(async move {
            let _permit = gate.acquire_owned().await.expect("semaphore closed");
            fetch::cached(&http, &url, &path).await.map(|_| ())
        });
    }
    while let Some(done) = jobs.join_next().await {
        done.map_err(|e| Error::BadData(format!("download task failed: {e}")))??;
    }
    Ok(paths)
}

/// Decode each cached LAZ tile and fold its points into the grid.
fn build_grid(spec: Spec, paths: &[PathBuf], source: DsmSource) -> Result<DsmGrid, Error> {
    let mut data = spec.empty();
    for p in paths {
        let points = read_laz(std::fs::read(p)?)?;
        grid::accumulate_max(&spec, &mut data, points);
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
        data: grid::fill_gaps(&spec, &data),
    })
}

/// All points of one LAZ file (EPT tiles are small: ~10k-200k points).
pub fn read_laz(bytes: Vec<u8>) -> Result<Vec<grid::Point>, Error> {
    let pd = las::Reader::new(std::io::Cursor::new(bytes))?.read_all()?;
    let xyz = pd.x().zip(pd.y()).zip(pd.z());
    Ok(xyz.zip(pd.classification()).map(|(((x, y), z), class)| grid::Point { x, y, z, class }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_city_sized_requests() {
        assert!(matches!(get_dsm(40.0, -74.0, 50_000.0).await, Err(Error::TooLarge(_))));
        assert!(matches!(get_dsm(40.0, -74.0, 0.0).await, Err(Error::TooLarge(_))));
    }

    /// Live: downloads ~20 MB the first time. `cargo test -p lidar -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn princeton_has_real_buildings() {
        let g = get_dsm(40.3487, -74.6593, 150.0).await.unwrap();
        assert_eq!(g.meta.source.provider, "usgs-3dep");
        let filled = g.data.iter().filter(|&&v| v != NODATA).count() as f64 / g.data.len() as f64;
        assert!(filled > 0.95, "only {filled:.2} of cells have a height");
        // the ground on campus is ~60-66 m above sea level; roofs and trees stand well above it
        let mut v: Vec<f32> = g.data.iter().copied().filter(|&v| v != NODATA).collect();
        v.sort_by(f32::total_cmp);
        let (low, high) = (v[v.len() / 20], v[v.len() * 99 / 100]);
        assert!((55.0..70.0).contains(&low), "ground {low}");
        assert!(high - low > 15.0, "tallest {high} vs ground {low}");
    }
}

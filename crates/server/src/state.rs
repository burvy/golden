//! Shared server state: height maps kept in memory between calls, built scenes, the CoStar kit, an HTTP
//! client, and the `ApiError` response every endpoint uses.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use contracts::{DsmGrid, EARTH_R};

use crate::heights::{self, HeightsError};

/// Height maps are fetched at least this big (metres), and rounded up to a multiple of it, so nearby
/// spots and the search area share one download.
pub const GRID_STEP_M: f64 = 250.0;
/// Keep at most this many height maps in memory (a 1.5 km one at 1 m cells is ~36 MB).
pub const MAX_GRIDS: usize = 4;

pub struct AppState {
    pub grids: Grids,
    /// Scene binaries by id (also written to `cache/scene/<id>.bin`).
    pub scenes: tokio::sync::Mutex<HashMap<String, Arc<Vec<u8>>>>,
    /// The CoStar kit if `COSTAR_KIT_DIR` is set and loads.
    pub kit: Option<&'static costar::Kit>,
    pub http: reqwest::Client,
    pub cache_root: PathBuf,
}

impl AppState {
    pub fn from_env() -> AppState {
        let kit = match costar::shared_kit() {
            Ok(k) => {
                eprintln!("CoStar kit: {} buildings", k.len());
                Some(k)
            }
            Err(e) => {
                eprintln!("CoStar kit not loaded ({e}): /api/buildings returns empty lists");
                None
            }
        };
        let http = reqwest::Client::builder()
            .user_agent("golden-hackathon/0.1")
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("http client");
        AppState {
            grids: Grids::default(),
            scenes: Default::default(),
            kit,
            http,
            cache_root: vexcel::cache::default_root(),
        }
    }
}

/// An error response: `{"error": "..."}` with a status (docs/contracts.md "Errors").
#[derive(Debug, Clone)]
pub struct ApiErr(pub StatusCode, pub String);

impl ApiErr {
    pub fn bad(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::NOT_FOUND, msg.into())
    }
    pub fn unprocessable(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::UNPROCESSABLE_ENTITY, msg.into())
    }
    pub fn upstream(msg: impl Into<String>) -> ApiErr {
        ApiErr(StatusCode::BAD_GATEWAY, msg.into())
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(contracts::ApiError { error: self.1 })).into_response()
    }
}

impl From<HeightsError> for ApiErr {
    fn from(e: HeightsError) -> ApiErr {
        match &e {
            HeightsError::NoData { lidar: lidar::Error::Offline(_), .. } => crate::guard::offline_err("no height map here"),
            HeightsError::NoData { .. } => ApiErr::unprocessable(format!("no height data here ({e})")),
            HeightsError::Vexcel(vexcel::Error::NoKey) => {
                eprintln!("height map: {e}");
                ApiErr(StatusCode::SERVICE_UNAVAILABLE, "no height map is cached here, and this server can't download one".into())
            }
            HeightsError::Vexcel(vexcel::Error::TooLarge(_)) => ApiErr::bad(e.to_string()),
            HeightsError::Vexcel(_) => {
                eprintln!("height map: {e}");
                ApiErr::upstream("height map download failed")
            }
        }
    }
}

/// Height maps in memory: the server's only grid cache. A request is served from any held grid that covers
/// its circle. Otherwise it joins a download already under way whose square will cover it, or starts one
/// (Vexcel, else USGS LiDAR) that later requests can join. So two calls for the same new area download it
/// once, and a call for a held area never waits for someone else's download: the lock only guards the lists
/// and is never held across an `.await`.
pub struct Grids {
    inner: Mutex<GridsInner>,
    fetch: Fetch,
}

/// Builds a grid for (lat, lon, radius_m). The real one is `heights::get_dsm`; tests pass their own.
pub type Fetch = Arc<dyn Fn(f64, f64, f64) -> Pin<Box<dyn Future<Output = Result<DsmGrid, ApiErr>> + Send>> + Send + Sync>;

/// One download's result, shared by everyone waiting for it.
type Shared = Arc<tokio::sync::OnceCell<Result<Arc<DsmGrid>, ApiErr>>>;

#[derive(Default)]
struct GridsInner {
    /// Newest first, at most `MAX_GRIDS`.
    held: Vec<Arc<DsmGrid>>,
    pending: Vec<Pending>,
}

/// A download under way: the square of side `2 * radius_m` around (lat, lon) it will cover.
struct Pending {
    lat: f64,
    lon: f64,
    radius_m: f64,
    result: Shared,
}

impl Default for Grids {
    fn default() -> Grids {
        Grids::with_fetch(Arc::new(|lat, lon, radius_m| {
            Box::pin(async move { heights::get_dsm(lat, lon, radius_m).await.map_err(ApiErr::from) })
        }))
    }
}

impl Grids {
    /// Grids that build missing areas with `fetch` instead of downloading (for tests).
    pub fn with_fetch(fetch: Fetch) -> Grids {
        Grids { inner: Mutex::default(), fetch }
    }

    /// Hold `grid` as if it had been downloaded (tests insert synthetic grids this way).
    #[cfg(test)]
    pub fn insert(&self, grid: DsmGrid) {
        self.inner.lock().unwrap().keep(Arc::new(grid));
    }

    /// A grid covering `need_m` metres around (lat, lon).
    pub async fn around(&self, lat: f64, lon: f64, need_m: f64) -> Result<Arc<DsmGrid>, ApiErr> {
        let (area, result) = {
            let mut inner = self.inner.lock().unwrap();
            if let Some(g) = inner.held.iter().find(|g| covers(g, lat, lon, need_m)) {
                return Ok(g.clone());
            }
            match inner.pending.iter().find(|p| square_covers(p.lat, p.lon, p.radius_m, lat, lon, need_m)) {
                Some(p) => ((p.lat, p.lon, p.radius_m), p.result.clone()),
                None => {
                    let p = Pending { lat, lon, radius_m: fetch_radius(need_m), result: Shared::default() };
                    let joined = ((p.lat, p.lon, p.radius_m), p.result.clone());
                    inner.pending.push(p);
                    joined
                }
            }
        };
        // Whoever gets here first runs the download; the rest wait for its result. If that caller goes away
        // (the client hung up), the next waiter runs it instead.
        let (alat, alon, aradius) = area;
        let got = result.get_or_init(|| async { (self.fetch)(alat, alon, aradius).await.map(Arc::new) }).await.clone();
        self.inner.lock().unwrap().finish(&result, &got);
        got
    }
}

impl GridsInner {
    fn keep(&mut self, g: Arc<DsmGrid>) {
        self.held.insert(0, g);
        self.held.truncate(MAX_GRIDS);
    }

    /// The first waiter back moves a finished download out of `pending` (into `held` if it worked; after an
    /// error the next request tries again).
    fn finish(&mut self, result: &Shared, got: &Result<Arc<DsmGrid>, ApiErr>) {
        if let Some(i) = self.pending.iter().position(|p| Arc::ptr_eq(&p.result, result)) {
            self.pending.remove(i);
            if let Ok(g) = got {
                self.keep(g.clone());
            }
        }
    }
}

/// Will a download of the square of side `2 * radius_m` around (clat, clon) hold the circle of `need_m`
/// around (lat, lon)? (`heights::get_dsm` promises at least that square.)
fn square_covers(clat: f64, clon: f64, radius_m: f64, lat: f64, lon: f64, need_m: f64) -> bool {
    let east = (lon - clon).to_radians() * EARTH_R * clat.to_radians().cos();
    let north = (lat - clat).to_radians() * EARTH_R;
    east.abs() + need_m <= radius_m && north.abs() + need_m <= radius_m
}

/// Round up to the next `GRID_STEP_M`, within the 3 km fetch limit.
pub fn fetch_radius(need_m: f64) -> f64 {
    ((need_m / GRID_STEP_M).ceil() * GRID_STEP_M).clamp(GRID_STEP_M, vexcel::MAX_RADIUS_M)
}

/// Does `g` hold the whole circle of `need_m` around (lat, lon)?
pub fn covers(g: &DsmGrid, lat: f64, lon: f64, need_m: f64) -> bool {
    let (c, r) = g.cell_of(lat, lon);
    let k = need_m / g.meta.cell_m;
    c - k >= 0.0 && r - k >= 0.0 && c + k <= f64::from(g.meta.width) && r + k <= f64::from(g.meta.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{DsmMeta, DsmSource, NODATA};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn grid(radius_m: f64) -> DsmGrid {
        let n = (2.0 * radius_m) as u32;
        let source = DsmSource { provider: "synthetic".into(), collection: None, captured_from: None, captured_to: None, native_res_m: 1.0 };
        let meta = DsmMeta { center_lat: 40.0, center_lon: -74.0, cell_m: 1.0, width: n, height: n, nodata: NODATA, source };
        DsmGrid { meta, data: vec![0.0; (n * n) as usize] }
    }

    #[test]
    fn covers_only_circles_inside_the_grid() {
        let g = grid(500.0);
        assert!(covers(&g, 40.0, -74.0, 500.0));
        assert!(!covers(&g, 40.0, -74.0, 501.0));
        // 300 m north of the centre, a 150 m circle fits; a 250 m one doesn't
        let lat = 40.0 + (300.0f64 / contracts::EARTH_R).to_degrees();
        assert!(covers(&g, lat, -74.0, 150.0));
        assert!(!covers(&g, lat, -74.0, 250.0));
    }

    #[test]
    fn a_download_covers_circles_inside_its_square() {
        assert!(square_covers(40.0, -74.0, 500.0, 40.0, -74.0, 500.0));
        assert!(!square_covers(40.0, -74.0, 500.0, 40.0, -74.0, 501.0));
        let north = |m: f64| 40.0 + (m / contracts::EARTH_R).to_degrees();
        assert!(square_covers(40.0, -74.0, 500.0, north(300.0), -74.0, 199.9));
        assert!(!square_covers(40.0, -74.0, 500.0, north(300.0), -74.0, 200.1));
    }

    /// A fetch that counts its calls and builds a grid of the asked size, once `gate` says go.
    fn gated_fetch(calls: Arc<AtomicUsize>, gate: Arc<tokio::sync::Semaphore>) -> Fetch {
        Arc::new(move |lat, lon, radius_m| {
            let (calls, gate) = (calls.clone(), gate.clone());
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                gate.acquire().await.unwrap().forget();
                let mut g = grid(radius_m);
                (g.meta.center_lat, g.meta.center_lon) = (lat, lon);
                Ok(g)
            })
        })
    }

    const WAIT: Duration = Duration::from_secs(5);

    async fn until_called(calls: &AtomicUsize) {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_held_area_is_not_blocked_by_a_pending_download() {
        let (calls, gate) = (Arc::new(AtomicUsize::new(0)), Arc::new(tokio::sync::Semaphore::new(0)));
        let grids = Arc::new(Grids::with_fetch(gated_fetch(calls.clone(), gate.clone())));
        grids.insert(grid(500.0)); // centred on 40, -74
        let g = grids.clone();
        let far = tokio::spawn(async move { g.around(34.0, -118.0, 300.0).await });
        until_called(&calls).await;
        // The LA download is stuck on the gate, and the held grid still answers at once.
        let near = tokio::time::timeout(WAIT, grids.around(40.0, -74.0, 400.0)).await.expect("blocked by the download");
        assert_eq!(near.unwrap().meta.center_lat, 40.0);
        gate.add_permits(1);
        assert_eq!(far.await.unwrap().unwrap().meta.center_lat, 34.0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn requests_for_one_new_area_share_one_download() {
        let (calls, gate) = (Arc::new(AtomicUsize::new(0)), Arc::new(tokio::sync::Semaphore::new(0)));
        let grids = Arc::new(Grids::with_fetch(gated_fetch(calls.clone(), gate.clone())));
        let ask = |lat: f64, need: f64| {
            let g = grids.clone();
            tokio::spawn(async move { g.around(lat, -74.0, need).await })
        };
        let first = ask(40.0, 900.0); // downloads 1000 m around 40, -74
        until_called(&calls).await;
        let same = ask(40.0, 900.0);
        let inside = ask(40.0 + (200.0f64 / contracts::EARTH_R).to_degrees(), 500.0); // 200 m north, inside the square
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        gate.add_permits(1);
        for h in [first, same, inside] {
            assert_eq!(tokio::time::timeout(WAIT, h).await.unwrap().unwrap().unwrap().meta.center_lat, 40.0);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one download for all three");
        // And it is held now: no new download.
        grids.around(40.0, -74.0, 100.0).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_download_is_shared_then_retried() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let fails: Fetch = Arc::new(move |_, _, _| {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Err(ApiErr::upstream("height map download failed"))
            })
        });
        let grids = Grids::with_fetch(fails);
        let (a, b) = tokio::join!(grids.around(40.0, -74.0, 100.0), grids.around(40.0, -74.0, 100.0));
        assert_eq!(a.unwrap_err().0, StatusCode::BAD_GATEWAY);
        assert_eq!(b.unwrap_err().0, StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "both waited on one try");
        assert!(grids.around(40.0, -74.0, 100.0).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2, "an error isn't kept");
    }

    #[test]
    fn holds_at_most_max_grids() {
        let grids = Grids::with_fetch(gated_fetch(Arc::default(), Arc::new(tokio::sync::Semaphore::new(0))));
        for _ in 0..MAX_GRIDS + 2 {
            grids.insert(grid(10.0));
        }
        assert_eq!(grids.inner.lock().unwrap().held.len(), MAX_GRIDS);
    }

    #[test]
    fn fetch_radius_rounds_up_and_caps() {
        assert_eq!(fetch_radius(1.0), 250.0);
        assert_eq!(fetch_radius(1001.0), 1250.0);
        assert_eq!(fetch_radius(9999.0), 3000.0);
    }
}

//! Shared server state: height maps kept in memory between calls, built scenes, the CoStar kit, an HTTP
//! client, and the `ApiError` response every endpoint uses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use contracts::DsmGrid;

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
#[derive(Debug)]
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
            HeightsError::NoData { .. } => ApiErr::unprocessable(format!("no height data here ({e})")),
            HeightsError::Vexcel(vexcel::Error::NoKey) => ApiErr(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
            HeightsError::Vexcel(vexcel::Error::TooLarge(_)) => ApiErr::bad(e.to_string()),
            HeightsError::Vexcel(_) => {
                eprintln!("height map: {e}");
                ApiErr::upstream("height map download failed")
            }
        }
    }
}

/// Height maps in memory. A request is served from any held grid that covers its circle; otherwise one
/// is fetched (Vexcel, else USGS LiDAR) and kept. One fetch at a time, so two calls for the same new area
/// don't download it twice.
#[derive(Default)]
pub struct Grids {
    held: tokio::sync::Mutex<Vec<Arc<DsmGrid>>>,
}

impl Grids {
    /// A grid covering `need_m` metres around (lat, lon).
    pub async fn around(&self, lat: f64, lon: f64, need_m: f64) -> Result<Arc<DsmGrid>, ApiErr> {
        let mut held = self.held.lock().await;
        if let Some(g) = held.iter().find(|g| covers(g, lat, lon, need_m)) {
            return Ok(g.clone());
        }
        let radius = fetch_radius(need_m);
        let g = Arc::new(heights::get_dsm(lat, lon, radius).await?);
        held.insert(0, g.clone());
        held.truncate(MAX_GRIDS);
        Ok(g)
    }
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
    fn fetch_radius_rounds_up_and_caps() {
        assert_eq!(fetch_radius(1.0), 250.0);
        assert_eq!(fetch_radius(1001.0), 1250.0);
        assert_eq!(fetch_radius(9999.0), 3000.0);
    }
}

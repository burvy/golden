//! Fixture answers, so the web app can be built against every endpoint before the real one exists.
//!
//! - Endpoints with no real version yet (`scene`, `photo`, `agent`) always answer with their fixture.
//! - With `GOLDEN_STUBS=1`, the real ones (`sun`, `horizon`, `search`, `buildings`) do too: no Vexcel key,
//!   CoStar kit or network needed.
//!
//! Every fixture answer has the header `x-golden-stub: <fixture file>`, so the web app can tell.
//! The fixtures are compiled in from `fixtures/`, the same files `cargo test -p contracts` checks.

use std::sync::OnceLock;

use axum::{
    extract::{rejection::JsonRejection, Path, Query},
    http::{header, HeaderName, HeaderValue},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use contracts::AgentRequest;
use serde::Deserialize;

use crate::api::{check_lat_lon, check_range, ApiErr, ApiResult};

pub const SUN_PATH: Fixture = Fixture::json("sun_path.json", include_str!("../../../fixtures/sun_path.json"));
pub const HORIZON: Fixture = Fixture::json("horizon_profile.json", include_str!("../../../fixtures/horizon_profile.json"));
pub const SEARCH: Fixture = Fixture::json("search_result.json", include_str!("../../../fixtures/search_result.json"));
pub const BUILDINGS: Fixture = Fixture::json("buildings.json", include_str!("../../../fixtures/buildings.json"));
pub const SCENE_META: Fixture = Fixture::json("scene_meta.json", include_str!("../../../fixtures/scene_meta.json"));
pub const PHOTO_META: Fixture = Fixture::json("photo_meta.json", include_str!("../../../fixtures/photo_meta.json"));
pub const CALL_SHEET: Fixture = Fixture::json("call_sheet.json", include_str!("../../../fixtures/call_sheet.json"));
const SCENE_BIN: &[u8] = include_bytes!("../../../fixtures/scene.bin");

pub struct Fixture {
    pub file: &'static str,
    pub body: &'static str,
}

impl Fixture {
    const fn json(file: &'static str, body: &'static str) -> Fixture {
        Fixture { file, body }
    }
}

impl IntoResponse for &Fixture {
    fn into_response(self) -> Response {
        ([(header::CONTENT_TYPE, "application/json")], stub_header(self.file), self.body).into_response()
    }
}

fn stub_header(file: &'static str) -> [(HeaderName, HeaderValue); 1] {
    [(HeaderName::from_static("x-golden-stub"), HeaderValue::from_static(file))]
}

/// `GOLDEN_STUBS=1` (or `true`): real endpoints answer with fixtures too.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| matches!(std::env::var("GOLDEN_STUBS").as_deref(), Ok("1" | "true")))
}

pub fn routes() -> Router {
    Router::new()
        .route("/api/scene", get(scene))
        .route("/api/scene/{file}", get(scene_bin))
        .route("/api/photo", get(photo))
        .route("/api/photo/{file}", get(photo_jpg))
        .route("/api/agent", post(agent))
}

// ---- /api/scene ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // read once the real scene lands
struct SceneParams {
    lat: f64,
    lon: f64,
    #[serde(default = "default_scene_radius")]
    radius_m: f64,
    #[serde(default)]
    detail: Detail,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum Detail {
    #[default]
    Phone,
    Desktop,
}

fn default_scene_radius() -> f64 {
    1000.0
}

async fn scene(q: Result<Query<SceneParams>, axum::extract::rejection::QueryRejection>) -> ApiResult<Response> {
    let Query(q) = q?;
    check_lat_lon(q.lat, q.lon)?;
    check_range("radius_m", q.radius_m, 100.0, 3000.0)?;
    Ok(SCENE_META.into_response())
}

/// `{id}.bin`: the fixture's binary, for the fixture's id only.
async fn scene_bin(Path(file): Path<String>) -> ApiResult<Response> {
    let id = fixture_id(SCENE_META.body);
    if file != format!("{id}.bin") {
        return Err(ApiErr::not_found(format!("no scene {file}")));
    }
    let headers = [(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")];
    Ok((headers, stub_header("scene.bin"), SCENE_BIN).into_response())
}

// ---- /api/photo ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct PhotoParams {
    lat: f64,
    lon: f64,
    #[serde(default)]
    kind: Option<String>,
    look_az_deg: Option<f64>,
    #[serde(default = "default_photo_radius")]
    radius_m: f64,
    #[serde(default = "default_max_px")]
    max_px: f64,
}

fn default_photo_radius() -> f64 {
    150.0
}
fn default_max_px() -> f64 {
    1024.0
}

async fn photo(q: Result<Query<PhotoParams>, axum::extract::rejection::QueryRejection>) -> ApiResult<Response> {
    let Query(q) = q?;
    check_lat_lon(q.lat, q.lon)?;
    if !matches!(q.kind.as_deref(), None | Some("oblique" | "ortho")) {
        return Err(ApiErr::bad("kind must be \"oblique\" or \"ortho\""));
    }
    check_range("radius_m", q.radius_m, 25.0, 500.0)?;
    check_range("max_px", q.max_px, 256.0, 2048.0)?;
    Ok(PHOTO_META.into_response())
}

/// There is no fixture image, so the stub's `image_url` is a 404 until the real photo endpoint lands.
async fn photo_jpg(Path(file): Path<String>) -> ApiErr {
    ApiErr::not_found(format!("no photo {file} (the photo endpoint is still a stub with no image)"))
}

// ---- /api/agent ---------------------------------------------------------------------------

async fn agent(body: Result<Json<AgentRequest>, JsonRejection>) -> ApiResult<Response> {
    let Json(req) = body?;
    if req.text.trim().is_empty() {
        return Err(ApiErr::bad("text is empty"));
    }
    Ok(CALL_SHEET.into_response())
}

fn fixture_id(json: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(json).expect("fixture is JSON");
    v["id"].as_str().unwrap_or_default().to_string()
}

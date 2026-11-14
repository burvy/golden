//! golden HTTP API on http://localhost:8787 (docs/contracts.md has every endpoint and shape).
//!
//!   GET  /                       the web app (`web/`, override with `GOLDEN_WEB_DIR`)
//!   GET  /api/health             "ok"
//!   GET  /api/sun                SunPath: sunrise, sunset, golden hours, the sun's track
//!   GET  /api/horizon            HorizonProfile: skyline from a point, when buildings hide the sun
//!   POST /api/search             SearchResult: best spots in an area for a Brief
//!   GET  /api/scene (+ .bin)     SceneMeta: heights in rings for the 3D view
//!   GET  /api/photo (+ .jpg)     PhotoMeta: Vexcel oblique looking toward the sun
//!   GET  /api/buildings[/{id}]   CoStar buildings (empty without the kit)
//!   POST /api/agent              CallSheet: typed brief in, call sheet out (Gemini, or the offline mock)
//!   GET  /api/tiles/vexcel/...   Vexcel map tiles (proxy + cache), 204 where none
//!   GET  /proto/map              map prototype
//!
//! Heights come from Vexcel where our key covers (LA, Houston, Princeton...), else USGS LiDAR (`heights.rs`).
//! Binds 127.0.0.1:8787; set `GOLDEN_ADDR=0.0.0.0:8787` to try it from a phone on the same Wi-Fi.

use std::sync::Arc;

use axum::Router;
use tower_http::services::ServeDir;

mod agent;
mod api;
mod heights;
mod mesh;
mod scene;
mod secrets;
mod state;
mod sunpath;
mod tiles;
mod tz;

const DEFAULT_ADDR: &str = "127.0.0.1:8787";

#[tokio::main]
async fn main() {
    let addr = std::env::var("GOLDEN_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("could not bind {addr} (already running?): {e}"));
    let state = Arc::new(state::AppState::from_env());
    println!("Gemini: {}", if secrets::get("GEMINI_API_KEY").is_some() { "key found" } else { "no key: the agent runs as the offline mock" });
    println!("golden on http://{addr}  (web app at /, API under /api)");
    axum::serve(listener, app(state)).await.expect("server crashed");
}

fn app(state: Arc<state::AppState>) -> Router {
    let web = std::env::var("GOLDEN_WEB_DIR").unwrap_or_else(|_| "web".to_string());
    Router::new()
        .route("/api/health", axum::routing::get(|| async { "ok" }))
        .merge(api::routes(state.clone()))
        .merge(agent::routes(state.clone()))
        .merge(mesh::routes(state))
        .merge(tiles::routes())
        .fallback_service(ServeDir::new(web))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(uri: &str) -> (StatusCode, serde_json::Value) {
        let res = app(Arc::new(state::AppState::from_env())).oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    async fn post(uri: &str, body: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::post(uri).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        let res = app(Arc::new(state::AppState::from_env())).oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn sun_endpoint_returns_the_contract_sun_path() {
        let (status, body) = call("/api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21").await;
        assert_eq!(status, StatusCode::OK);
        let p: contracts::SunPath = serde_json::from_value(body).unwrap();
        assert!(p.sunset.as_ref().unwrap().t.starts_with("2026-06-21T20:07"), "{:?}", p.sunset);
        assert!((p.sunset.unwrap().az_deg - 299.33).abs() < 0.05);
        assert_eq!(p.tz, "America/Los_Angeles");
    }

    #[tokio::test]
    async fn sun_endpoint_rejects_bad_input() {
        assert_eq!(call("/api/sun?lat=34&lon=-118&date=June").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/sun?lat=134&lon=-118&date=2026-06-21").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/sun?lat=34&lon=-118&date=2026-06-21&typo=1").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/sun?lat=34&lon=-118&date=2026-06-21&tz=Nowhere").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/sun?lat=78.2&lon=15.6&date=2026-12-21&tz=Arctic/Longyearbyen").await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn errors_are_api_error_json() {
        let (status, body) = call("/api/horizon?lat=34&lon=-118&radius_m=99999").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("radius_m"));
        let (status, body) = post("/api/search", r#"{"area":{"lat":34,"lon":-118,"radius_m":300},"date":"2026-06-21","oops":1}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].is_string());
        assert_eq!(post("/api/agent", r#"{"text":"  "}"#).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/scene/zzz.bin").await.0, StatusCode::NOT_FOUND);
        assert_eq!(call("/api/photo/nonsense.jpg").await.0, StatusCode::NOT_FOUND);
    }
}

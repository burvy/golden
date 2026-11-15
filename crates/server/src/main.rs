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
//!   POST /api/agent              CallSheet: typed brief in, call sheet out (Claude, or the offline mock)
//!   POST /api/chat               ChatReply: the brief as a chat (asks up to 3 questions, then searches)
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
#[cfg(test)]
mod testkit;
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
    println!("Claude: {}", if secrets::get("ANTHROPIC_API_KEY").is_some() { "key found" } else { "no key (ANTHROPIC_API_KEY / ANTHROPIC_ENV_FILE): the agent runs as the offline mock" });
    println!(
        "Vexcel heights: {}",
        if vexcel::auth::ApiKey::from_env().is_ok() {
            "key found"
        } else {
            "no key (VEXCEL_API_KEY / VEXCEL_ENV_FILE): cache only (cached ~0.9 m tiles stand in for missing 6 cm ones); USGS LiDAR where nothing is cached"
        }
    );
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

    // ---- happy paths, fully offline: a synthetic grid in `Grids`, no keys, no network, no demo cache ----

    async fn send(st: &Arc<state::AppState>, req: Request<Body>) -> (StatusCode, axum::body::Bytes) {
        let res = app(st.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        (status, res.into_body().collect().await.unwrap().to_bytes())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(st: &Arc<state::AppState>, uri: &str) -> T {
        let (status, bytes) = send(st, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {}", String::from_utf8_lossy(&bytes));
        serde_json::from_slice(&bytes).unwrap()
    }

    fn brief(radius_m: f64, standing: &str, framing: &str) -> String {
        serde_json::json!({
            "text": "", "area": {"lat": testkit::LAT, "lon": testkit::LON, "radius_m": radius_m},
            "date": "2026-06-21", "tz": "America/Los_Angeles", "event": "sunset",
            "standing": standing, "framing": framing, "max_spots": 5
        })
        .to_string()
    }

    #[tokio::test]
    async fn horizon_sees_the_wall_and_when_it_hides_the_sun() {
        let st = Arc::new(testkit::state(testkit::city()));
        let uri = format!("/api/horizon?lat={}&lon={}&step_deg=1&date=2026-06-21&tz=America/Los_Angeles", testkit::LAT, testkit::LON);
        let p: contracts::HorizonProfile = get_json(&st, &uri).await;
        assert_eq!(p.points.len(), 360);
        let at = |az: f64| p.points.iter().find(|q| (q.az_deg - az).abs() < 0.01).unwrap();
        // The wall: 80 m tall, 200 m west, eye 1.7 m up -> about 21 deg; open to the east (only towers far off).
        assert!((at(270.0).elev_deg - 21.4).abs() < 1.0, "{:?}", at(270.0));
        assert!((at(270.0).dist_m.unwrap() - 200.0).abs() < 10.0);
        assert!(at(90.0).elev_deg < 1.0);
        let sun = p.sun.expect("date given, so the sun block is filled");
        let hidden = sun.hidden_at.clone().expect("the wall hides the June sunset");
        assert!(hidden.as_str() < sun.flat_sunset.as_deref().unwrap());
        assert!((sun.hidden_elev_deg.unwrap() - 20.0).abs() < 3.0, "{sun:?}");
        assert_eq!(sun.golden_min_visible, 0, "hidden well before golden hour (6 deg)");
    }

    #[tokio::test]
    async fn search_returns_ranked_spots_and_a_capped_heatmap() {
        let st = Arc::new(testkit::state(testkit::city()));
        let req = Request::post("/api/search").header("content-type", "application/json").body(Body::from(brief(400.0, "any", "clear"))).unwrap();
        let (status, bytes) = send(&st, req).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
        let r: contracts::SearchResult = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r.brief.area.radius_m, 400.0);
        assert!(!r.spots.is_empty() && r.spots.len() <= 5, "{} spots", r.spots.len());
        assert!(r.spots.windows(2).all(|w| w[0].score >= w[1].score), "ranked best first");
        assert!(r.searched > 0);
        let h = &r.heat;
        assert!(h.width <= contracts::search::MAX_HEAT_SIDE && h.height <= contracts::search::MAX_HEAT_SIDE);
        assert_eq!(h.values.len(), (h.width * h.height) as usize);
        assert!(h.values.iter().any(Option::is_some));
        // one layer for the implicit sun goal, the fixed funnel stages, the spacing
        assert_eq!(r.layers.len(), 1);
        assert_eq!(r.layers[0].goal, "sun_clear");
        assert_eq!(r.stages.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Points in the area", "Somewhere to stand"]);
        assert_eq!(r.stages[1].n, r.searched);
        assert!(r.spacing_m.unwrap() >= 40.0);
    }

    #[tokio::test]
    async fn search_takes_edited_goal_formulas() {
        // what the page sends after a formula edit: the brief with its goals
        let st = Arc::new(testkit::state(testkit::city()));
        let mut b: serde_json::Value = serde_json::from_str(&brief(300.0, "any", "clear")).unwrap();
        b["goals"] = serde_json::json!([
            {"use": "sun_clear"},
            {"def": {"name": "low", "about": "Under 20 m: easy to reach", "score": {"op": "lt", "x": {"f": "ground_m"}, "than": {"num": 20}}}, "must": true}
        ]);
        let req = Request::post("/api/search").header("content-type", "application/json").body(Body::from(b.to_string())).unwrap();
        let (status, bytes) = send(&st, req).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
        let r: contracts::SearchResult = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(r.layers.iter().map(|l| (l.goal.as_str(), l.must)).collect::<Vec<_>>(), [("sun_clear", false), ("low", true)]);
        assert_eq!(r.stages.last().unwrap().goal.as_deref(), Some("low"));
        assert_eq!(r.stages.last().unwrap().label, "Under 20 m");
        assert!(r.spots.iter().all(|s| s.ground_m < 20.0));
    }

    #[tokio::test]
    async fn search_outside_the_held_grid_fails_without_downloading() {
        let st = Arc::new(testkit::state(testkit::city()));
        let body = brief(400.0, "any", "clear").replace("34.05", "40.35");
        let req = Request::post("/api/search").header("content-type", "application/json").body(Body::from(body)).unwrap();
        let (status, bytes) = send(&st, req).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{}", String::from_utf8_lossy(&bytes));
    }

    #[tokio::test]
    async fn scene_meta_and_binary_agree() {
        let st = Arc::new(testkit::state(testkit::city()));
        let uri = format!("/api/scene?lat={}&lon={}&radius_m=500&detail=phone", testkit::LAT, testkit::LON);
        let meta: contracts::SceneMeta = get_json(&st, &uri).await;
        assert_eq!(meta.data_url, format!("/api/scene/{}.bin", meta.id));
        assert_eq!(meta.bytes, meta.rings.iter().map(|r| r.byte_len()).sum::<u64>());
        // Served from memory, and from the disk copy after a restart (a fresh state with the same cache dir).
        let (status, bin) = send(&st, Request::get(&meta.data_url).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bin.len() as u64, meta.bytes);
        let fresh = Arc::new(state::AppState { cache_root: st.cache_root.clone(), ..testkit::state(testkit::grid(&[])) });
        let (status, again) = send(&fresh, Request::get(&meta.data_url).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again, bin);
        std::fs::remove_dir_all(&st.cache_root).ok();
    }
}

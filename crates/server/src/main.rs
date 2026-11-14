//! golden HTTP API on http://localhost:8787. Shapes and endpoints: docs/contracts.md.
//!
//!   GET  /api/health                     "ok"
//!   GET  /api/sun                        SunPath (core's NOAA sun, local times, golden hour)
//!   GET  /api/horizon                    HorizonProfile (height map -> core horizon + sunblock)
//!   POST /api/search                     SearchResult (height map of area + 1500 m -> core search)
//!   GET  /api/buildings, /api/buildings/{id}   CoStar buildings (kit in COSTAR_KIT_DIR; 503 without it)
//!   GET  /api/scene, /api/photo, POST /api/agent   fixture stubs until the real ones land (see stubs.rs)
//!   GET  /api/tiles/vexcel/{z}/{x}/{y}   Vexcel aerial map tile (proxy + cache), 204 where none
//!   GET  /proto/map                      map prototype
//!
//! `GOLDEN_ADDR` overrides the address (default 127.0.0.1:8787). `GOLDEN_STUBS=1` makes every endpoint answer with its fixture (no keys, kit or network needed).
//! Errors are `{"error": "..."}` with the statuses in docs/contracts.md. CORS is open so the web app can be
//! served from another local port; the server only listens on 127.0.0.1.

use axum::{
    http::StatusCode,
    routing::{get, post},
    Router,
};
use tower_http::cors::CorsLayer;

mod api;
mod buildings;
mod engine;
mod heights;
mod stubs;
mod sun_path;
mod tiles;

use api::ApiErr;

const ADDR: &str = "127.0.0.1:8787";

#[tokio::main]
async fn main() {
    let addr = std::env::var("GOLDEN_ADDR").unwrap_or_else(|_| ADDR.to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("could not bind {addr} (already running?): {e}"));
    if stubs::on() {
        println!("GOLDEN_STUBS=1: every endpoint answers with its fixture");
    } else {
        // Load the CoStar kit in the background so the first /api/buildings is fast.
        tokio::task::spawn_blocking(|| match costar::shared_kit() {
            Ok(kit) => println!("CoStar kit: {} buildings", kit.len()),
            Err(e) => eprintln!("CoStar kit not loaded ({e}): /api/buildings will answer 503"),
        });
    }
    println!("golden server on http://{addr}  (try /api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21)");
    axum::serve(listener, app()).await.expect("server crashed");
}

fn app() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/sun", get(sun_path::get_sun))
        .route("/api/horizon", get(engine::get_horizon))
        .route("/api/search", post(engine::post_search))
        .route("/api/buildings", get(buildings::get_buildings))
        .route("/api/buildings/{id}", get(buildings::get_building))
        .merge(stubs::routes())
        .merge(tiles::routes())
        .fallback(|| async { ApiErr(StatusCode::NOT_FOUND, "no such endpoint (see docs/contracts.md)".into()) })
        .layer(CorsLayer::permissive())
}

async fn index() -> &'static str {
    "golden API (docs/contracts.md)

GET  /api/health
GET  /api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21
GET  /api/horizon?lat=34.0490&lon=-118.2510&date=2026-06-21
POST /api/search   (body: Brief)
GET  /api/buildings?lat=34.0490&lon=-118.2510
GET  /api/buildings/{id}
GET  /api/scene, /api/photo, POST /api/agent   (fixture stubs)
GET  /proto/map
"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn send(req: Request<Body>) -> (StatusCode, Option<String>, Vec<u8>) {
        let res = app().oneshot(req).await.unwrap();
        let status = res.status();
        let stub = res.headers().get("x-golden-stub").map(|v| v.to_str().unwrap().to_string());
        (status, stub, res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn get_json(uri: &str) -> (StatusCode, serde_json::Value) {
        let (status, _, body) = send(Request::get(uri).body(Body::empty()).unwrap()).await;
        (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn sun_returns_the_contract_sun_path() {
        let (status, body) = get_json("/api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21").await;
        assert_eq!(status, StatusCode::OK);
        let p: contracts::SunPath = serde_json::from_value(body).unwrap();
        assert_eq!(p.tz, "America/Los_Angeles");
        assert!(p.sunset.unwrap().t.starts_with("2026-06-21T20:07:4"));
    }

    #[tokio::test]
    async fn bad_input_is_a_json_error() {
        for (uri, want) in [
            ("/api/sun?lat=34&lon=-118&date=June", StatusCode::BAD_REQUEST),
            ("/api/sun?lat=134&lon=-118&date=2026-06-21", StatusCode::BAD_REQUEST),
            ("/api/sun?lat=34&lon=-118&date=2026-06-21&typo=1", StatusCode::BAD_REQUEST),
            ("/api/sun?lat=34&lon=-118&date=2026-06-21&tz=Nowhere", StatusCode::BAD_REQUEST),
            ("/api/sun?lat=69.6&lon=18.9&date=2026-06-21&tz=Europe/Oslo", StatusCode::UNPROCESSABLE_ENTITY),
            ("/api/horizon?lat=34&lon=-118&radius_m=99999", StatusCode::BAD_REQUEST),
            ("/api/buildings?lat=34&lon=-118&limit=0", StatusCode::BAD_REQUEST),
            ("/api/nope", StatusCode::NOT_FOUND),
        ] {
            let (status, body) = get_json(uri).await;
            assert_eq!(status, want, "{uri}");
            let e: contracts::ApiError = serde_json::from_value(body).unwrap_or_else(|_| panic!("{uri}: not an ApiError"));
            assert!(!e.error.is_empty());
        }
    }

    #[tokio::test]
    async fn search_rejects_a_bad_brief() {
        let post = |body: &str| {
            Request::post("/api/search").header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())).unwrap()
        };
        let too_big = r#"{"area":{"lat":34.049,"lon":-118.251,"radius_m":5000},"date":"2026-06-21"}"#;
        assert_eq!(send(post(too_big)).await.0, StatusCode::BAD_REQUEST);
        let typo = r#"{"area":{"lat":34.049,"lon":-118.251,"radius_m":300},"date":"2026-06-21","framming":"between"}"#;
        assert_eq!(send(post(typo)).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stubs_answer_with_their_fixtures() {
        let (status, stub, body) = send(Request::get("/api/scene?lat=34.049&lon=-118.251").body(Body::empty()).unwrap()).await;
        assert_eq!((status, stub.as_deref()), (StatusCode::OK, Some("scene_meta.json")));
        let meta: contracts::SceneMeta = serde_json::from_slice(&body).unwrap();

        let (status, _, bin) = send(Request::get(meta.data_url.as_str()).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bin.len() as u64, meta.bytes as u64);
        assert_eq!(get_json("/api/scene/nope.bin").await.0, StatusCode::NOT_FOUND);

        let (status, body) = get_json("/api/photo?lat=34.049&lon=-118.251&look_az_deg=290").await;
        assert_eq!(status, StatusCode::OK);
        let _: contracts::PhotoMeta = serde_json::from_value(body).unwrap();

        let req = Request::post("/api/agent")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"text":"sun between two towers"}"#))
            .unwrap();
        let (status, stub, body) = send(req).await;
        assert_eq!((status, stub.as_deref()), (StatusCode::OK, Some("call_sheet.json")));
        let _: contracts::CallSheet = serde_json::from_slice(&body).unwrap();
    }
}

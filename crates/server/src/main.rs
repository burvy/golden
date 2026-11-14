//! golden HTTP API on http://localhost:8787.
//!
//! Endpoints so far:
//!   GET /                                   plain-text index (web/ will replace this)
//!   GET /api/health                         "ok"
//!   GET /api/sun?lat=..&lon=..&date=YYYY-MM-DD
//!       200 {"time":"2026-06-22T03:07:41Z","azimuth_deg":299.33}
//!       400 bad/missing query params, 422 no sunset that day (polar day/night)
//!   GET /api/tiles/vexcel/{z}/{x}/{y}       Vexcel aerial map tile (proxy + cache), 204 where none
//!   GET /proto/map                          map prototype: Esri map/photo + Vexcel + day/night shading + time slider

use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{NaiveDate, SecondsFormat};
use golden_core::sun::{self, NoSunset};
use serde::{Deserialize, Serialize};

mod heights;
mod tiles;

const ADDR: &str = "127.0.0.1:8787";

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind(ADDR)
        .await
        .unwrap_or_else(|e| panic!("could not bind {ADDR} (already running?): {e}"));
    println!("golden server on http://localhost:8787  (try /api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21)");
    axum::serve(listener, app()).await.expect("server crashed");
}

fn app() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/sun", get(get_sun))
        .merge(tiles::routes())
}

async fn index() -> &'static str {
    "golden API\n\nGET /api/health\nGET /api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21\n"
}

#[derive(Deserialize)]
struct SunQuery {
    lat: f64,
    lon: f64,
    /// Local calendar date at the spot, YYYY-MM-DD.
    date: NaiveDate,
}

#[derive(Serialize)]
struct SunResponse {
    /// ISO 8601 UTC, e.g. "2026-06-22T03:07:41Z".
    time: String,
    azimuth_deg: f64,
}

async fn get_sun(Query(q): Query<SunQuery>) -> Response {
    if !(-90.0..=90.0).contains(&q.lat) || !(-180.0..=180.0).contains(&q.lon) {
        return error(StatusCode::BAD_REQUEST, "lat must be in [-90, 90] and lon in [-180, 180]");
    }
    match sun::sunset(q.lat, q.lon, q.date) {
        Ok(s) => Json(SunResponse {
            time: s.time.to_rfc3339_opts(SecondsFormat::Secs, true),
            azimuth_deg: (s.azimuth_deg * 100.0).round() / 100.0,
        })
        .into_response(),
        Err(NoSunset::PolarDay) => error(StatusCode::UNPROCESSABLE_ENTITY, "no sunset: the sun stays up all day"),
        Err(NoSunset::PolarNight) => error(StatusCode::UNPROCESSABLE_ENTITY, "no sunset: the sun never rises"),
    }
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(uri: &str) -> (StatusCode, serde_json::Value) {
        let res = app().oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn sun_endpoint_returns_la_solstice_sunset() {
        let (status, body) = call("/api/sun?lat=34.0522&lon=-118.2437&date=2026-06-21").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["time"], "2026-06-22T03:07:41Z");
        assert!((body["azimuth_deg"].as_f64().unwrap() - 299.335).abs() < 0.01);
    }

    #[tokio::test]
    async fn sun_endpoint_rejects_bad_input() {
        assert_eq!(call("/api/sun?lat=34&lon=-118&date=June").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/sun?lat=134&lon=-118&date=2026-06-21").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("/api/sun?lat=69.6&lon=18.9&date=2026-06-21").await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

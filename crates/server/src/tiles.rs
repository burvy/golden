//! Map tiles for the web app, and the map prototype page.
//!
//! The phone never sees the Vexcel key: it asks us for `/api/tiles/vexcel/{z}/{x}/{y}` and we fetch,
//! cache and pass the tile on. Where Vexcel has nothing for us (outside LA / Houston / Princeton for our key)
//! we answer 204, and MapLibre shows the Esri imagery layer underneath.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use vexcel::ortho::{OrthoTile, OrthoTiles};

pub fn routes() -> Router {
    let tiles = match OrthoTiles::from_env() {
        Ok(t) => {
            if !t.has_key() {
                eprintln!("no Vexcel key (VEXCEL_API_KEY / VEXCEL_ENV_FILE): map tiles come from the cache only");
            }
            Some(Arc::new(t))
        }
        Err(e) => {
            eprintln!("Vexcel tiles disabled: {e}");
            None
        }
    };
    Router::new()
        .route("/api/tiles/vexcel/{z}/{x}/{y}", get(vexcel_tile))
        .route("/proto/map", get(|| async { Html(include_str!("../static/proto_map.html")) }))
        .with_state(tiles)
}

async fn vexcel_tile(
    State(tiles): State<Option<Arc<OrthoTiles>>>,
    Path((z, x, y)): Path<(u8, u32, String)>,
) -> Response {
    // accept "123" or "123.jpg"
    let Ok(y) = y.split('.').next().unwrap_or_default().parse::<u32>() else {
        return error(StatusCode::BAD_REQUEST, "tile y must be a number");
    };
    let Some(tiles) = tiles else { return StatusCode::NO_CONTENT.into_response() };
    match tiles.get(z, x, y).await {
        Ok(OrthoTile::Image { bytes, content_type }) => (
            [(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "public, max-age=604800")],
            bytes,
        )
            .into_response(),
        Ok(OrthoTile::Empty) => (StatusCode::NO_CONTENT, [(header::CACHE_CONTROL, "public, max-age=86400")]).into_response(),
        Err(vexcel::Error::BadData(msg)) => error(StatusCode::BAD_REQUEST, &msg),
        Err(e) => {
            eprintln!("vexcel tile {z}/{x}/{y}: {e}");
            error(StatusCode::BAD_GATEWAY, "Vexcel tile fetch failed")
        }
    }
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

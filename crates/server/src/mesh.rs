//! Vexcel's textured 3D city mesh for the web app's 3D view, passed through with our key (cached on disk by
//! `vexcel::mesh`).
//!
//! - `GET /api/mesh/at?lat=&lon=` -> `{"collection", "tileset_url", "attribution"}`, or 404 where Vexcel has no mesh
//!   (the app then uses the height-map view).
//! - `GET /api/mesh/tileset.json?...`, `GET /api/mesh/tile.glb?...` -> the Vexcel response, child URLs pointing back here.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use tokio::sync::OnceCell;
use vexcel::auth::ApiKey;

use crate::state::{AppState, ApiErr};

pub const PREFIX: &str = "/api/mesh/";
/// How far around the spot the 3D view loads the mesh (m). Beyond it: sky and haze.
pub const NEAR_M: f64 = 3000.0;
pub const ATTRIBUTION: &str = "3D mesh © Vexcel Imaging US, Inc. · © Microsoft";

struct Mesh {
    app: Arc<AppState>,
    key: Option<ApiKey>,
    collections: OnceCell<serde_json::Value>,
}

pub fn routes(app: Arc<AppState>) -> Router {
    let key = vexcel::auth::ApiKey::from_env().ok();
    let st = Arc::new(Mesh { app, key, collections: OnceCell::new() });
    Router::new()
        .route("/api/mesh/at", get(at))
        .route("/api/mesh/{kind}", get(pass))
        .route("/api/mesh/near/{lat}/{lon}/{r}/{kind}", get(pass_near))
        .with_state(st)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AtQ {
    lat: f64,
    lon: f64,
}

async fn at(State(st): State<Arc<Mesh>>, q: Result<Query<AtQ>, axum::extract::rejection::QueryRejection>) -> Result<Response, ApiErr> {
    let Query(q) = q.map_err(|e| ApiErr(StatusCode::BAD_REQUEST, e.body_text()))?;
    let key = st.key.as_ref().ok_or_else(|| ApiErr(StatusCode::NOT_FOUND, "no Vexcel key: no 3D mesh".into()))?;
    let fc = st
        .collections
        .get_or_try_init(|| async {
            let b = vexcel::mesh::get(&st.app.http, key, &st.app.cache_root, "collections", "", PREFIX).await?;
            serde_json::from_slice(&b).map_err(|e| vexcel::Error::BadData(e.to_string()))
        })
        .await
        .map_err(|e| ApiErr(StatusCode::BAD_GATEWAY, format!("Vexcel 3D coverage: {e}")))?;
    let Some(c) = vexcel::mesh::covering(fc, q.lat, q.lon) else {
        return Err(ApiErr(StatusCode::NOT_FOUND, "Vexcel has no 3D mesh here".into()));
    };
    Ok(Json(serde_json::json!({
        "collection": c,
        "tileset_url": format!("{PREFIX}near/{:.5}/{:.5}/{NEAR_M}/tileset.json?collection={c}&layer=3d-cities", q.lat, q.lon),
        "attribution": ATTRIBUTION,
    }))
    .into_response())
}

async fn pass(State(st): State<Arc<Mesh>>, Path(kind): Path<String>, RawQuery(q): RawQuery) -> Result<Response, ApiErr> {
    serve(&st, &kind, q, None).await
}

/// Same, with tilesets trimmed to `r` metres around (lat, lon); children keep the prefix since their URIs are relative.
async fn pass_near(State(st): State<Arc<Mesh>>, Path((lat, lon, r, kind)): Path<(f64, f64, f64, String)>, RawQuery(q): RawQuery) -> Result<Response, ApiErr> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || !(50.0..=20000.0).contains(&r) {
        return Err(ApiErr(StatusCode::BAD_REQUEST, "near: lat, lon or radius out of range".into()));
    }
    serve(&st, &kind, q, Some((lat, lon, r))).await
}

async fn serve(st: &Mesh, kind: &str, q: Option<String>, near: Option<(f64, f64, f64)>) -> Result<Response, ApiErr> {
    let kind = vexcel::mesh::strip_ext(kind).to_string();
    if !vexcel::mesh::allowed(&kind) || kind == "collections" {
        return Err(ApiErr(StatusCode::NOT_FOUND, format!("unknown mesh resource {kind:?}")));
    }
    let key = st.key.as_ref().ok_or_else(|| ApiErr(StatusCode::NOT_FOUND, "no Vexcel key".into()))?;
    let q = q.unwrap_or_default();
    let prefix = match near {
        Some((lat, lon, r)) => format!("{PREFIX}near/{lat}/{lon}/{r}/"),
        None => PREFIX.to_string(),
    };
    let body = vexcel::mesh::get(&st.app.http, key, &st.app.cache_root, &kind, &q, &prefix).await.map_err(|e| match e {
        vexcel::Error::Api { status: 404, .. } => ApiErr(StatusCode::NOT_FOUND, "no such mesh tile".into()),
        e => {
            eprintln!("vexcel mesh {kind}: {e}");
            ApiErr(StatusCode::BAD_GATEWAY, "Vexcel mesh fetch failed".into())
        }
    })?;
    let body = match near {
        Some((lat, lon, r)) if kind == "tileset" => vexcel::mesh::trim_tileset(&body, lat, lon, r),
        _ => body,
    };
    let ct = if kind == "tile" { "model/gltf-binary" } else { "application/json" };
    Ok(([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "public, max-age=604800")], body).into_response())
}

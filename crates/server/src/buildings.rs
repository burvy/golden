//! `GET /api/buildings` and `GET /api/buildings/{id}`: CoStar buildings from the `costar` crate
//! (the kit in `COSTAR_KIT_DIR`, loaded once, ~0.3 s). The kit never leaves this machine except as these answers.

use axum::{
    extract::{rejection::QueryRejection, Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use contracts::{Building, BuildingList};
use costar::Kit;
use serde::Deserialize;

use crate::api::{check_lat_lon, check_range, ApiErr, ApiResult};
use crate::stubs;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingsParams {
    lat: f64,
    lon: f64,
    #[serde(default = "d_radius")]
    radius_m: f64,
    #[serde(default = "d_limit")]
    limit: u32,
}

fn d_radius() -> f64 {
    150.0
}
fn d_limit() -> u32 {
    20
}

pub async fn get_buildings(q: Result<Query<BuildingsParams>, QueryRejection>) -> ApiResult<Response> {
    let Query(q) = q?;
    check_lat_lon(q.lat, q.lon)?;
    check_range("radius_m", q.radius_m, 1.0, 2000.0)?;
    check_range("limit", q.limit as f64, 1.0, 100.0)?;
    if stubs::on() {
        return Ok(stubs::BUILDINGS.into_response());
    }
    let kit = kit().await?;
    Ok(Json(near(kit, &q)).into_response())
}

pub async fn get_building(Path(id): Path<String>) -> ApiResult<Response> {
    if stubs::on() {
        let list: BuildingList = serde_json::from_str(stubs::BUILDINGS.body).expect("fixture parses");
        let b = list.buildings.into_iter().find(|b| b.id == id);
        return b.map(|b| Json(b).into_response()).ok_or_else(|| unknown(&id));
    }
    one(kit().await?, &id).map(|b| Json(b).into_response())
}

pub fn near(kit: &Kit, q: &BuildingsParams) -> BuildingList {
    let mut buildings = kit.buildings_near(q.lat, q.lon, q.radius_m);
    buildings.truncate(q.limit as usize);
    BuildingList { buildings }
}

pub fn one(kit: &Kit, id: &str) -> ApiResult<Building> {
    kit.building(id).ok_or_else(|| unknown(id))
}

fn unknown(id: &str) -> ApiErr {
    ApiErr::not_found(format!("unknown building {id:?} (ids look like costar:1234567)"))
}

/// The shared kit. 503 if the server wasn't given one: the rest of the API still works without it.
async fn kit() -> ApiResult<&'static Kit> {
    let loaded = tokio::task::spawn_blocking(costar::shared_kit)
        .await
        .map_err(|e| ApiErr(StatusCode::INTERNAL_SERVER_ERROR, format!("internal error: {e}")))?;
    loaded.map_err(|e| {
        eprintln!("CoStar kit: {e}");
        ApiErr(StatusCode::SERVICE_UNAVAILABLE, format!("CoStar data not available on this server: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MADE-UP one-building kit (never real CoStar rows in git).
    fn tiny_kit() -> (tempfile::TempDir, Kit) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let rows = [
            r#"{"id":1,"primaryAddress":{"latitude":34.0490,"longitude":-118.2510,"city":"Los Angeles","state":"CA"},"buildingName":"Pretend Plaza","contacts":[]}"#,
            r#"{"id":2,"primaryAddress":{"latitude":34.0495,"longitude":-118.2510,"city":"Los Angeles","state":"CA"},"contacts":[]}"#,
        ];
        std::fs::write(dir.path().join("data/commercial-property.jsonl"), rows.join("\n")).unwrap();
        let kit = Kit::load(dir.path()).unwrap();
        (dir, kit)
    }

    fn params(radius_m: f64, limit: u32) -> BuildingsParams {
        BuildingsParams { lat: 34.0490, lon: -118.2510, radius_m, limit }
    }

    #[test]
    fn nearest_first_and_limited() {
        let (_d, kit) = tiny_kit();
        let all = near(&kit, &params(150.0, 20)).buildings;
        assert_eq!(all.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["costar:1", "costar:2"]);
        assert_eq!(near(&kit, &params(150.0, 1)).buildings.len(), 1);
        assert!(near(&kit, &params(10.0, 20)).buildings.len() == 1);
    }

    #[test]
    fn one_by_id_or_404() {
        let (_d, kit) = tiny_kit();
        assert_eq!(one(&kit, "costar:1").unwrap().name.as_deref(), Some("Pretend Plaza"));
        assert_eq!(one(&kit, "costar:999").unwrap_err().0, StatusCode::NOT_FOUND);
    }
}

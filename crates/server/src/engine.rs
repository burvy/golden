//! `GET /api/horizon` and `POST /api/search`: height map (Vexcel, else USGS LiDAR; see `heights`) ->
//! burvy's core (`golden_core::horizon`, `sunblock`, `search`) -> contract JSON.
//!
//! The first call for a new area downloads the height map (cached under `cache/` after that), so it can take
//! seconds to a minute. Core's maths runs on a blocking thread so it doesn't stall other requests.

use std::time::Instant;

use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        Query,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use contracts::search::{DEFAULT_TZ, MAX_BRIEF_RADIUS_M};
use contracts::{Brief, DsmGrid, Event, HorizonProfile, SearchResult, Standing};
use golden_core::horizon::{horizon, HorizonError, HorizonQuery};
use golden_core::search::{search, SearchError, SearchOutput, MAX_SPOTS, SKYLINE_RADIUS_M};
use golden_core::sunblock::sun_block;
use serde::Deserialize;

use crate::api::{check_lat_lon, check_range, parse_date, parse_tz, utc_offset_s, ApiErr, ApiResult};
use crate::heights::{self, HeightsError};
use crate::stubs;

/// A roof spot belongs to the nearest CoStar building within this distance (CoStar's point is the
/// address, usually inside or at the edge of the footprint).
const ROOF_BUILDING_M: f64 = 50.0;

// ---- /api/horizon ------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorizonParams {
    lat: f64,
    lon: f64,
    #[serde(default = "d_above")]
    above_m: f64,
    #[serde(default = "d_radius")]
    radius_m: f64,
    #[serde(default = "d_step")]
    step_deg: f64,
    #[serde(default)]
    az_from: f64,
    #[serde(default = "d_az_to")]
    az_to: f64,
    date: Option<String>,
    #[serde(default = "d_tz")]
    tz: String,
}

fn d_above() -> f64 {
    1.7
}
fn d_radius() -> f64 {
    1500.0
}
fn d_step() -> f64 {
    1.0
}
fn d_az_to() -> f64 {
    360.0
}
fn d_tz() -> String {
    DEFAULT_TZ.to_string()
}

pub async fn get_horizon(q: Result<Query<HorizonParams>, QueryRejection>) -> ApiResult<Response> {
    let Query(q) = q?;
    check_lat_lon(q.lat, q.lon)?;
    check_range("above_m", q.above_m, 0.0, 500.0)?;
    check_range("radius_m", q.radius_m, 50.0, 3000.0)?;
    check_range("step_deg", q.step_deg, 0.25, 10.0)?;
    check_range("az_from", q.az_from, -360.0, 720.0)?;
    check_range("az_to", q.az_to, -360.0, 720.0)?;
    let tz = parse_tz(&q.tz)?;
    let date = q.date.as_deref().map(parse_date).transpose()?;
    if stubs::on() {
        return Ok(stubs::HORIZON.into_response());
    }

    let grid = heights::get_dsm(q.lat, q.lon, q.radius_m).await.map_err(heights_err)?;
    let hq = HorizonQuery {
        lat: q.lat,
        lon: q.lon,
        above_m: q.above_m,
        radius_m: q.radius_m,
        step_deg: q.step_deg,
        az_from: q.az_from,
        az_to: q.az_to,
    };
    let sun = date.map(|d| (d, utc_offset_s(tz, d)));
    let profile = blocking(move || horizon_profile(&grid, &hq, sun)).await??;
    Ok(Json(profile).into_response())
}

/// The skyline, plus when it hides the evening sun if `sun` (local date, UTC offset) is given.
pub fn horizon_profile(grid: &DsmGrid, q: &HorizonQuery, sun: Option<(chrono::NaiveDate, i32)>) -> ApiResult<HorizonProfile> {
    let mut profile = horizon(grid, q).map_err(|e| match e {
        HorizonError::BadQuery(_) => ApiErr::bad(e.to_string()),
        HorizonError::ObserverOffGrid | HorizonError::ObserverNoData => ApiErr::impossible(format!("no height data there: {e}")),
    })?;
    if let Some((date, offset)) = sun {
        profile.sun = Some(sun_block(&profile.points, q.lat, q.lon, date, offset, Event::Sunset));
    }
    Ok(profile)
}

// ---- /api/search -------------------------------------------------------------------------

pub async fn post_search(body: Result<Json<Brief>, JsonRejection>) -> ApiResult<Response> {
    let started = Instant::now();
    let Json(brief) = body?;
    check_lat_lon(brief.area.lat, brief.area.lon)?;
    check_range("area.radius_m", brief.area.radius_m, 50.0, MAX_BRIEF_RADIUS_M)?;
    check_range("max_spots", brief.max_spots as f64, 1.0, MAX_SPOTS as f64)?;
    let date = parse_date(&brief.date)?;
    let offset = utc_offset_s(parse_tz(&brief.tz)?, date);
    if stubs::on() {
        return Ok(stubs::SEARCH.into_response());
    }

    // The area, plus room to see the skyline from its edge.
    let grid = heights::get_dsm(brief.area.lat, brief.area.lon, brief.area.radius_m + SKYLINE_RADIUS_M)
        .await
        .map_err(heights_err)?;
    let (brief, out) = blocking(move || {
        let out = search(&grid, &brief, offset);
        (brief, out)
    })
    .await?;
    let mut out = out.map_err(search_err)?;
    tag_roof_buildings(&mut out).await;
    let SearchOutput { spots, heat, searched } = out;
    let took_ms = started.elapsed().as_millis() as u64;
    Ok(Json(SearchResult { brief, spots, heat, searched, took_ms }).into_response())
}

fn search_err(e: SearchError) -> ApiErr {
    match e {
        SearchError::BadDate(_) | SearchError::BadArea => ApiErr::bad(e.to_string()),
        SearchError::AreaOffGrid => ApiErr::impossible(format!("no height data there: {e}")),
        SearchError::NoSunEvent => ApiErr::impossible(e.to_string()),
    }
}

/// Roof spots get the CoStar building they're on (who to ask for access). Skipped without a kit.
async fn tag_roof_buildings(out: &mut SearchOutput) {
    let Ok(Ok(kit)) = tokio::task::spawn_blocking(costar::shared_kit).await else { return };
    for spot in out.spots.iter_mut().filter(|s| s.standing == Standing::Roof) {
        spot.building_id = kit.buildings_near(spot.lat, spot.lon, ROOF_BUILDING_M).into_iter().next().map(|b| b.id);
    }
}

// ---- shared -------------------------------------------------------------------------------

fn heights_err(e: HeightsError) -> ApiErr {
    match e {
        HeightsError::NoData { .. } => ApiErr::impossible(e.to_string()),
        HeightsError::Vexcel(vexcel::Error::TooLarge(_)) => ApiErr::bad(e.to_string()),
        HeightsError::Vexcel(_) => {
            eprintln!("height map: {e}");
            ApiErr(StatusCode::BAD_GATEWAY, format!("Vexcel failed: {e}"))
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> ApiResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiErr(StatusCode::INTERNAL_SERVER_ERROR, format!("internal error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture_grid() -> DsmGrid {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dsm_grid.json");
        vexcel::grid::read_files(&p).unwrap()
    }

    fn query(lat: f64, lon: f64) -> HorizonQuery {
        HorizonQuery { lat, lon, above_m: 1.7, radius_m: 100.0, step_deg: 10.0, az_from: 0.0, az_to: 360.0 }
    }

    #[test]
    fn horizon_on_the_fixture_grid_with_sun() {
        let g = fixture_grid();
        let june21 = chrono::NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        let p = horizon_profile(&g, &query(34.049, -118.251), Some((june21, -7 * 3600))).unwrap();
        assert_eq!(p.points.len(), 36);
        assert!((p.observer.eye_m - p.observer.ground_m - 1.7).abs() < 1e-9);
        let sun = p.sun.clone().unwrap();
        assert_eq!(sun.date, "2026-06-21");
        assert!(sun.flat_sunset.unwrap().starts_with("2026-06-21T20:0"));
        // and it serialises as the contract shape
        let back: HorizonProfile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.points.len(), 36);
    }

    #[test]
    fn horizon_off_the_grid_is_422() {
        let err = horizon_profile(&fixture_grid(), &query(35.0, -118.251), None).unwrap_err();
        assert_eq!(err.0, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn search_errors_map_to_statuses() {
        assert_eq!(search_err(SearchError::AreaOffGrid).0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(search_err(SearchError::BadArea).0, StatusCode::BAD_REQUEST);
    }
}

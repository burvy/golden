//! The data endpoints of docs/contracts.md: sun, horizon, search, scene, photo, buildings.
//! Handlers check parameters, fetch what they need (height maps through `state::Grids`), and call the pure
//! code in `core` / `sunpath` / `scene`. Heavy maths runs on the blocking pool.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use contracts::{Brief, BuildingList, Event, PhotoKind, PhotoMeta, SearchResult, Standing};
use golden_core::horizon::{horizon, HorizonQuery};
use golden_core::search::{skyline_need_m, SKYLINE_RADIUS_M};
use golden_core::sunblock::sun_block;
use serde::Deserialize;

use crate::state::{ApiErr, AppState};
use crate::{scene, sunpath, tz};

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/sun", get(get_sun))
        .route("/api/horizon", get(get_horizon))
        .route("/api/search", post(post_search))
        .route("/api/scene", get(get_scene))
        .route("/api/scene/{file}", get(get_scene_bin))
        .route("/api/photo", get(get_photo))
        .route("/api/photo/{file}", get(get_photo_jpg))
        .route("/api/buildings", get(get_buildings))
        .route("/api/buildings/{id}", get(get_building))
        .with_state(state)
}

type Q<T> = Result<Query<T>, QueryRejection>;

fn query<T>(q: Q<T>) -> Result<T, ApiErr> {
    q.map(|Query(t)| t).map_err(|e| ApiErr::bad(e.body_text()))
}

pub fn check_lat_lon(lat: f64, lon: f64) -> Result<(), ApiErr> {
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
        Ok(())
    } else {
        Err(ApiErr::bad("lat must be in [-90, 90] and lon in [-180, 180]"))
    }
}

fn in_range(name: &str, v: f64, lo: f64, hi: f64) -> Result<f64, ApiErr> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(ApiErr::bad(format!("{name} must be in [{lo}, {hi}]")))
    }
}

pub fn parse_date(s: &str) -> Result<NaiveDate, ApiErr> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| ApiErr::bad(format!("bad date {s:?}, expected YYYY-MM-DD")))
}

pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiErr> {
    tokio::task::spawn_blocking(f).await.map_err(|e| ApiErr::upstream(format!("worker failed: {e}")))
}

// ---- /api/sun ------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SunQ {
    lat: f64,
    lon: f64,
    date: String,
    tz: Option<String>,
    step_min: Option<u32>,
}

async fn get_sun(q: Q<SunQ>) -> Result<Response, ApiErr> {
    let q = query(q)?;
    check_lat_lon(q.lat, q.lon)?;
    let date = parse_date(&q.date)?;
    let zone = tz::parse(q.tz.as_deref().unwrap_or(tz::DEFAULT_TZ)).map_err(ApiErr::bad)?;
    let step = q.step_min.unwrap_or(10);
    if !(1..=60).contains(&step) {
        return Err(ApiErr::bad("step_min must be in [1, 60]"));
    }
    let p = sunpath::sun_path(q.lat, q.lon, date, zone, step);
    if p.sunrise.is_none() && p.sunset.is_none() {
        return Err(ApiErr::unprocessable("the sun doesn't rise or set there that day"));
    }
    Ok(Json(p).into_response())
}

// ---- /api/horizon ----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HorizonQ {
    lat: f64,
    lon: f64,
    above_m: Option<f64>,
    radius_m: Option<f64>,
    step_deg: Option<f64>,
    az_from: Option<f64>,
    az_to: Option<f64>,
    date: Option<String>,
    tz: Option<String>,
}

async fn get_horizon(State(st): State<Arc<AppState>>, q: Q<HorizonQ>) -> Result<Response, ApiErr> {
    let q = query(q)?;
    check_lat_lon(q.lat, q.lon)?;
    let hq = HorizonQuery {
        lat: q.lat,
        lon: q.lon,
        above_m: in_range("above_m", q.above_m.unwrap_or(1.7), 0.0, 500.0)?,
        radius_m: in_range("radius_m", q.radius_m.unwrap_or(SKYLINE_RADIUS_M), 50.0, 3000.0)?,
        step_deg: in_range("step_deg", q.step_deg.unwrap_or(1.0), 0.25, 10.0)?,
        az_from: q.az_from.unwrap_or(0.0),
        az_to: q.az_to.unwrap_or(360.0),
    };
    let sun = match &q.date {
        Some(d) => Some((parse_date(d)?, tz::parse(q.tz.as_deref().unwrap_or(tz::DEFAULT_TZ)).map_err(ApiErr::bad)?)),
        None => None,
    };
    let grid = st.grids.around(q.lat, q.lon, hq.radius_m).await?;
    let profile = blocking(move || {
        let mut p = horizon(&grid, &hq).map_err(|e| ApiErr::unprocessable(e.to_string()))?;
        if let Some((date, zone)) = sun {
            p.sun = Some(sun_block(&p.points, hq.lat, hq.lon, date, tz::offset_s(zone, date), Event::Sunset));
        }
        Ok::<_, ApiErr>(p)
    })
    .await??;
    Ok(Json(profile).into_response())
}

// ---- /api/search -----------------------------------------------------------------------------

async fn post_search(State(st): State<Arc<AppState>>, body: Result<Json<Brief>, JsonRejection>) -> Result<Response, ApiErr> {
    let Json(brief) = body.map_err(|e| ApiErr::bad(e.body_text()))?;
    Ok(Json(run_search(&st, brief).await?).into_response())
}

/// Search used by the endpoint and by the agent: the whole cost-function library (the seeds plus every
/// function the agent saved), and the objects of the kinds the goals name (none for a sun-only brief).
pub async fn run_search(st: &AppState, brief: Brief) -> Result<SearchResult, ApiErr> {
    let library = crate::agent::library::Library::at(&st.cache_root).all();
    let objects = crate::objects::for_goals(st, &brief.area, &brief.goals, &library).await;
    run_search_with(st, brief, library, objects).await
}

/// `run_search` with the cost functions the brief's `use` goals may name and the objects they may mean.
pub async fn run_search_with(st: &AppState, brief: Brief, library: Vec<contracts::CostFn>, objects: Vec<contracts::Object>) -> Result<SearchResult, ApiErr> {
    let t0 = Instant::now();
    check_lat_lon(brief.area.lat, brief.area.lon)?;
    in_range("area.radius_m", brief.area.radius_m, 50.0, contracts::search::MAX_BRIEF_RADIUS_M)?;
    if !(1..=50).contains(&brief.max_spots) {
        return Err(ApiErr::bad("max_spots must be in [1, 50]"));
    }
    let date = parse_date(&brief.date)?;
    let zone = tz::parse(&brief.tz).map_err(ApiErr::bad)?;
    let grid = st.grids.around(brief.area.lat, brief.area.lon, skyline_need_m(brief.area.radius_m)).await?;
    let b = brief.clone();
    let out = blocking(move || {
        let inputs = golden_core::search::Inputs { library: &library, objects: &objects };
        golden_core::search::search_with(&grid, &b, &inputs, tz::offset_s(zone, date))
    })
        .await?
        .map_err(|e| ApiErr::unprocessable(e.to_string()))?;
    let mut spots = out.spots;
    if let Some(kit) = st.kit {
        for s in spots.iter_mut().filter(|s| s.standing == Standing::Roof) {
            s.building_id = kit.buildings_near(s.lat, s.lon, 40.0).first().map(|b| b.id.clone());
        }
    }
    Ok(SearchResult {
        brief,
        spots,
        heat: out.heat,
        searched: out.searched,
        took_ms: t0.elapsed().as_millis() as u64,
        layers: out.layers,
        stages: out.stages,
        spacing_m: Some(out.spacing_m),
        objects: out.objects,
    })
}

// ---- /api/scene ------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneQ {
    lat: f64,
    lon: f64,
    radius_m: Option<f64>,
    detail: Option<String>,
}

async fn get_scene(State(st): State<Arc<AppState>>, q: Q<SceneQ>) -> Result<Response, ApiErr> {
    let q = query(q)?;
    check_lat_lon(q.lat, q.lon)?;
    let radius = in_range("radius_m", q.radius_m.unwrap_or(1000.0), 100.0, 3000.0)?;
    let px = match q.detail.as_deref().unwrap_or("phone") {
        "phone" => contracts::scene::PHONE_RING_PX,
        "desktop" => contracts::scene::DESKTOP_RING_PX,
        other => return Err(ApiErr::bad(format!("detail must be phone or desktop, not {other:?}"))),
    };
    let grid = st.grids.around(q.lat, q.lon, radius).await?;
    let built = blocking(move || scene::build(&grid, q.lat, q.lon, radius, px)).await?;
    let path = st.cache_root.join("scene").join(format!("{}.bin", built.meta.id));
    if !path.exists() {
        if let Some(d) = path.parent() {
            let _ = tokio::fs::create_dir_all(d).await;
        }
        let _ = tokio::fs::write(&path, &built.bytes).await;
    }
    crate::guard::bounded_insert(&mut *st.scenes.lock().await, built.meta.id.clone(), Arc::new(built.bytes), crate::guard::MAX_SCENES);
    Ok(Json(built.meta).into_response())
}

async fn get_scene_bin(State(st): State<Arc<AppState>>, Path(file): Path<String>) -> Result<Response, ApiErr> {
    let id = file.strip_suffix(".bin").unwrap_or(&file).to_string();
    if !id.chars().all(|c| c.is_ascii_hexdigit()) || id.is_empty() {
        return Err(ApiErr::not_found("unknown scene"));
    }
    let held = st.scenes.lock().await.get(&id).cloned();
    let bytes = match held {
        Some(b) => b.to_vec(),
        None => tokio::fs::read(st.cache_root.join("scene").join(format!("{id}.bin")))
            .await
            .map_err(|_| ApiErr::not_found("unknown scene"))?,
    };
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")],
        bytes,
    )
        .into_response())
}

// ---- /api/photo ------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhotoQ {
    lat: f64,
    lon: f64,
    kind: Option<String>,
    look_az_deg: Option<f64>,
    radius_m: Option<f64>,
    max_px: Option<u32>,
}

/// What a photo id encodes: the request, so `/api/photo/{id}.jpg` can rebuild it (Vexcel's answer is cached
/// on disk by `vexcel::get_oblique`). `o_<lat>_<lon>_<az>_<radius>_<px>`.
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoReq {
    pub lat: f64,
    pub lon: f64,
    pub az: f64,
    pub radius_m: u32,
    pub max_px: u32,
}

impl PhotoReq {
    pub fn id(&self) -> String {
        format!("o_{:.5}_{:.5}_{:.0}_{}_{}", self.lat, self.lon, self.az, self.radius_m, self.max_px)
    }
    pub fn parse(id: &str) -> Option<PhotoReq> {
        let p: Vec<&str> = id.strip_prefix("o_")?.split('_').collect();
        let [lat, lon, az, r, px] = p[..] else { return None };
        Some(PhotoReq { lat: lat.parse().ok()?, lon: lon.parse().ok()?, az: az.parse().ok()?, radius_m: r.parse().ok()?, max_px: px.parse().ok()? })
    }
    pub fn url(&self) -> String {
        format!("/api/photo/{}.jpg", self.id())
    }
    pub fn options(&self) -> vexcel::ObliqueOptions {
        let w = self.radius_m * 2;
        vexcel::ObliqueOptions {
            view_m: (w, w * 2 / 3),
            size_px: (self.max_px, self.max_px * 2 / 3),
            ..Default::default()
        }
    }
}

async fn get_photo(q: Q<PhotoQ>) -> Result<Response, ApiErr> {
    let q = query(q)?;
    check_lat_lon(q.lat, q.lon)?;
    if q.kind.as_deref().unwrap_or("oblique") != "oblique" {
        return Err(ApiErr::bad("only kind=oblique is available so far"));
    }
    let req = PhotoReq {
        lat: q.lat,
        lon: q.lon,
        az: in_range("look_az_deg", q.look_az_deg.unwrap_or(270.0), 0.0, 360.0)?,
        radius_m: in_range("radius_m", q.radius_m.unwrap_or(150.0), 25.0, 500.0)? as u32,
        max_px: in_range("max_px", f64::from(q.max_px.unwrap_or(1024)), 256.0, 2048.0)? as u32,
    };
    let o = fetch_oblique(&req).await?;
    Ok(Json(PhotoMeta {
        id: req.id(),
        kind: PhotoKind::Oblique,
        image_url: req.url(),
        width: o.width,
        height: o.height,
        look_az_deg: Some(o.look_az_deg),
        captured: None,
        attribution: "Imagery © Vexcel Imaging US, Inc.".into(),
    })
    .into_response())
}

async fn fetch_oblique(req: &PhotoReq) -> Result<vexcel::Oblique, ApiErr> {
    vexcel::get_oblique_with(req.lat, req.lon, req.az, &req.options()).await.map_err(|e| match e {
        vexcel::Error::NoKey if crate::guard::get().offline => crate::guard::offline_err("no aerial photo here"),
        vexcel::Error::NoKey => ApiErr(axum::http::StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        vexcel::Error::Api { status: 403 | 404, .. } => ApiErr::unprocessable("no Vexcel aerial photo here"),
        vexcel::Error::BadData(m) => ApiErr::bad(m),
        other => {
            eprintln!("oblique: {other}");
            ApiErr::upstream("Vexcel photo fetch failed")
        }
    })
}

async fn get_photo_jpg(Path(file): Path<String>) -> Result<Response, ApiErr> {
    let req = PhotoReq::parse(file.strip_suffix(".jpg").unwrap_or(&file)).ok_or_else(|| ApiErr::not_found("unknown photo"))?;
    let o = fetch_oblique(&req).await?;
    Ok(([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "public, max-age=604800")], o.jpeg).into_response())
}

// ---- /api/buildings --------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildingsQ {
    lat: f64,
    lon: f64,
    radius_m: Option<f64>,
    limit: Option<usize>,
}

async fn get_buildings(State(st): State<Arc<AppState>>, q: Q<BuildingsQ>) -> Result<Response, ApiErr> {
    let q = query(q)?;
    check_lat_lon(q.lat, q.lon)?;
    let radius = in_range("radius_m", q.radius_m.unwrap_or(150.0), 1.0, 2000.0)?;
    let limit = q.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(ApiErr::bad("limit must be in [1, 100]"));
    }
    let mut buildings = st.kit.map(|k| k.buildings_near(q.lat, q.lon, radius)).unwrap_or_default();
    buildings.truncate(limit);
    Ok(Json(BuildingList { buildings }).into_response())
}

async fn get_building(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response, ApiErr> {
    st.kit
        .and_then(|k| k.building(&id))
        .map(|b| Json(b).into_response())
        .ok_or_else(|| ApiErr::not_found(format!("unknown building {id}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_largest_search_gets_its_whole_skyline() {
        // A brief of the maximum radius still fits the 3 km fetch limit with the full skyline around it,
        // and the grid we'd fetch covers what core traces (no rays cut off at the grid edge).
        let need = skyline_need_m(contracts::search::MAX_BRIEF_RADIUS_M);
        assert!(need <= vexcel::MAX_RADIUS_M, "{need} m");
        assert!(crate::state::fetch_radius(need) >= need);
        assert!(skyline_need_m(300.0) >= 300.0 + golden_core::search::SKYLINE_RADIUS_M);
    }

    #[test]
    fn photo_ids_round_trip() {
        let r = PhotoReq { lat: 40.34871, lon: -74.65932, az: 247.0, radius_m: 150, max_px: 1024 };
        assert_eq!(r.id(), "o_40.34871_-74.65932_247_150_1024");
        assert_eq!(PhotoReq::parse(&r.id()), Some(r));
        assert_eq!(PhotoReq::parse("o_1_2_3"), None);
        assert_eq!(PhotoReq::parse("../etc"), None);
    }
}

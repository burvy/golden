//! One place that decides where a height map comes from, so the rest of the server just asks for a `DsmGrid`.
//!
//! Vexcel first (7.5 cm, recent, but our key only covers greater LA, Houston and Princeton). Where Vexcel refuses
//! (403 outside the geofence), has no key, or comes back mostly empty, use USGS 3DEP LiDAR (`lidar` crate:
//! US-wide, public, ~0.5 m, older surveys). `DsmMeta.source.provider` says which one answered.
//! Used by /api/horizon and /api/scene as they get wired up.
#![allow(dead_code)]

use contracts::{DsmGrid, NODATA};

#[derive(Debug, thiserror::Error)]
pub enum HeightsError {
    #[error("no height data here: Vexcel: {vexcel}; USGS LiDAR: {lidar}")]
    NoData { vexcel: String, lidar: lidar::Error },
    #[error("vexcel: {0}")]
    Vexcel(vexcel::Error),
}

pub async fn get_dsm(lat: f64, lon: f64, radius_m: f64) -> Result<DsmGrid, HeightsError> {
    let v = vexcel::get_dsm(lat, lon, radius_m).await;
    if !should_fall_back(&v) {
        return v.map_err(HeightsError::Vexcel);
    }
    let why = match &v {
        Ok(_) => "mostly empty".to_string(),
        Err(e) => e.to_string(),
    };
    lidar::get_dsm(lat, lon, radius_m).await.map_err(|lidar| HeightsError::NoData { vexcel: why, lidar })
}

/// Fall back when Vexcel can't serve this place; real failures (network, bad request) are reported as-is.
pub fn should_fall_back(v: &Result<DsmGrid, vexcel::Error>) -> bool {
    match v {
        Ok(g) => filled_share(g) < 0.5,
        Err(vexcel::Error::NoKey) => true,
        Err(vexcel::Error::Api { status, .. }) => matches!(status, 403 | 404),
        Err(_) => false,
    }
}

fn filled_share(g: &DsmGrid) -> f64 {
    if g.data.is_empty() {
        return 0.0;
    }
    g.data.iter().filter(|&&v| v != NODATA).count() as f64 / g.data.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{DsmMeta, DsmSource};

    fn grid(data: Vec<f32>) -> DsmGrid {
        let source = DsmSource {
            provider: "vexcel".into(),
            collection: None,
            captured_from: None,
            captured_to: None,
            native_res_m: 0.075,
        };
        let meta = DsmMeta { center_lat: 0.0, center_lon: 0.0, cell_m: 1.0, width: 2, height: 2, nodata: NODATA, source };
        DsmGrid { meta, data }
    }

    #[test]
    fn falls_back_outside_vexcel_coverage() {
        assert!(should_fall_back(&Err(vexcel::Error::Api { status: 403, body: "Forbidden".into() })));
        assert!(should_fall_back(&Err(vexcel::Error::NoKey)));
        assert!(should_fall_back(&Ok(grid(vec![NODATA, NODATA, NODATA, 5.0]))));
    }

    #[test]
    fn keeps_good_vexcel_data_and_real_errors() {
        assert!(!should_fall_back(&Ok(grid(vec![1.0, 2.0, 3.0, NODATA]))));
        assert!(!should_fall_back(&Err(vexcel::Error::Api { status: 400, body: "bad wkt".into() })));
        assert!(!should_fall_back(&Err(vexcel::Error::TooLarge(9e9))));
    }

    /// Live, end to end: Princeton is outside Vexcel's area, so this must come from USGS LiDAR, and
    /// core's horizon must work on it unchanged. `cargo test -p server -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn princeton_horizon_from_lidar() {
        let g = get_dsm(40.3487, -74.6593, 500.0).await.unwrap();
        assert_eq!(g.meta.source.provider, "usgs-3dep");
        // Cannon Green, behind Nassau Hall, standing eye height, looking toward the November sunset (~245 deg)
        let q = golden_core::horizon::HorizonQuery {
            lat: 40.34805, lon: -74.65895, above_m: 1.6, radius_m: 450.0, step_deg: 5.0, az_from: 225.0, az_to: 270.0,
        };
        let p = golden_core::horizon::horizon(&g, &q).unwrap();
        for pt in &p.points {
            println!("az {:5.1}  elev {:5.1} deg  at {:?} m", pt.az_deg, pt.elev_deg, pt.dist_m.map(|d| d.round()));
        }
        // on a tree-lined campus the western skyline is well above the true horizon
        assert!(p.points.iter().all(|pt| pt.elev_deg > 2.0));
    }
}

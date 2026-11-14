//! `HorizonProfile` (docs/contracts.md#horizonprofile): the skyline from one eye point.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorizonProfile {
    pub observer: Observer,
    pub radius_m: f64,
    pub step_deg: f64,
    pub points: Vec<HorizonPoint>,
    /// Only when the request had a `date`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sun: Option<SunBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observer {
    pub lat: f64,
    pub lon: f64,
    /// DSM surface under the observer (a roof if on a roof).
    pub ground_m: f64,
    /// `ground_m + above_m`.
    pub eye_m: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorizonPoint {
    pub az_deg: f64,
    /// Highest obstruction angle in this direction (negative if all below eye level).
    pub elev_deg: f64,
    /// Distance to that obstruction; None if no height data in this direction (then elev_deg is 0).
    pub dist_m: Option<f64>,
}

/// When buildings hide the sun from a point on one day. Shared by `HorizonProfile` and `Spot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SunBlock {
    pub date: String,
    /// Sunset (or sunrise) over a flat horizon.
    pub flat_sunset: Option<String>,
    /// Last moment the sun's centre is above the skyline (sunrise: first moment). None if nothing blocks it.
    pub hidden_at: Option<String>,
    pub hidden_az_deg: Option<f64>,
    pub hidden_elev_deg: Option<f64>,
    /// Minutes of golden hour with the sun in view.
    pub golden_min_visible: u32,
}

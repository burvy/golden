//! `SunPath` (docs/contracts.md#sunpath): the sun over one local day, flat horizon.
//! Times are ISO 8601 strings with the local offset; this crate has no clock or time-zone code.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SunPath {
    pub lat: f64,
    pub lon: f64,
    /// Local day, `YYYY-MM-DD`.
    pub date: String,
    /// IANA time zone the times are in, e.g. `America/Los_Angeles`.
    pub tz: String,
    pub sunrise: Option<SunEvent>,
    pub sunset: Option<SunEvent>,
    pub golden_hour: GoldenHour,
    pub points: Vec<SunPoint>,
}

/// Sunrise or sunset over a flat horizon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SunEvent {
    pub t: String,
    pub az_deg: f64,
}

/// Sun elevation between -4 and +6 degrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenHour {
    pub morning: Option<TimeRange>,
    pub evening: Option<TimeRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SunPoint {
    pub t: String,
    pub az_deg: f64,
    pub elev_deg: f64,
}

/// Golden hour bounds, in degrees of sun elevation.
pub const GOLDEN_HOUR_MIN_ELEV_DEG: f64 = -4.0;
pub const GOLDEN_HOUR_MAX_ELEV_DEG: f64 = 6.0;

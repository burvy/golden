//! `Brief`, `SearchResult`, `Spot` (docs/contracts.md#brief).

use serde::{Deserialize, Serialize};

use crate::goal::{Goal, Rated, DEFAULT_LENS_DEG};
use crate::horizon::SunBlock;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brief {
    /// As typed; for the agent and logs only.
    #[serde(default)]
    pub text: String,
    pub area: Area,
    /// Local day, `YYYY-MM-DD`.
    pub date: String,
    #[serde(default = "default_tz")]
    pub tz: String,
    #[serde(default)]
    pub event: Event,
    #[serde(default)]
    pub standing: Standing,
    #[serde(default)]
    pub framing: Framing,
    #[serde(default = "default_max_spots")]
    pub max_spots: u32,
    /// What the shot should do (docs/metrics.md v2). Empty = `[{"use": "sun_<framing>"}]`, i.e. the
    /// sunset search as it was before goals existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<Goal>,
    /// Horizontal lens angle for "in one frame" checks, degrees.
    #[serde(default = "default_lens_deg")]
    pub lens_deg: f64,
}

pub const DEFAULT_TZ: &str = "America/Los_Angeles";
pub const MAX_BRIEF_RADIUS_M: f64 = 800.0;

pub(crate) fn default_tz() -> String {
    DEFAULT_TZ.to_string()
}

fn default_max_spots() -> u32 {
    10
}

fn default_lens_deg() -> f64 {
    DEFAULT_LENS_DEG
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Area {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    #[default]
    Sunset,
    Sunrise,
}

/// Where the person stands. `Any` only appears in a `Brief`; a `Spot` is always ground or roof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    #[default]
    Any,
    Ground,
    Roof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framing {
    /// Sun visible down to the horizon, as long as possible.
    #[default]
    Clear,
    /// Sun sets into a gap between two buildings.
    Between,
    /// Sun sets behind a building (silhouette).
    Behind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    /// The request with defaults filled in.
    pub brief: Brief,
    pub spots: Vec<Spot>,
    pub heat: HeatGrid,
    pub searched: u32,
    pub took_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spot {
    pub id: String,
    pub rank: u32,
    /// 0-1, only comparable within one search.
    pub score: f64,
    pub lat: f64,
    pub lon: f64,
    pub standing: Standing,
    pub ground_m: f64,
    pub eye_m: f64,
    pub sun: SunBlock,
    /// Plain-English reasons, best first.
    pub why: Vec<String>,
    pub building_id: Option<String>,
    /// One entry per goal of the brief (the implicit sun goal when it had none), same order.
    #[serde(default)]
    pub scores: Vec<Rated>,
}

/// Coarse score map; same grid layout as `DsmGrid` (row 0 north, row-major).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatGrid {
    pub center_lat: f64,
    pub center_lon: f64,
    pub cell_m: f64,
    pub width: u32,
    pub height: u32,
    /// 0-1, None where nothing was searched. width * height values.
    pub values: Vec<Option<f32>>,
}

/// At most this many cells per side in `HeatGrid` (keeps the JSON small on a phone).
pub const MAX_HEAT_SIDE: u32 = 40;

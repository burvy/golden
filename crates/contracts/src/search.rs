//! `Brief`, `SearchResult`, `Spot` (docs/contracts.md#brief).

use serde::{Deserialize, Serialize};

use crate::goal::{Goal, Object, Rated, DEFAULT_LENS_DEG};
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
    /// For people only, e.g. "Downtown LA, around 5th & Flower". The search ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// One per goal of the brief (the implicit sun goal when it had none), same order: each goal's
    /// score at every candidate, so the browser can re-weight and toggle musts without a new search.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<GoalLayer>,
    /// The funnel, in order: how many candidates were left after each stage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<Stage>,
    /// Picked spots are at least this far apart, metres (the browser's re-pick uses the same).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing_m: Option<f64>,
    /// The objects the goals measured from (their `ObjRef`s, resolved), so the map can mark them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<Object>,
}

/// One goal's score at every candidate (docs/contracts.md#goal-layers-and-stages).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalLayer {
    /// `Goal::name()`.
    pub goal: String,
    /// The weight it was searched with: `Goal::weight_or_default()`.
    pub weight: f64,
    /// The goal is a must: a candidate scoring 0 on it is dropped.
    pub must: bool,
    /// The goal's score, 0..1, at each candidate (one per cell, same grid as `SearchResult.heat`).
    /// Raw, not normalised. `null` exactly where `SearchResult.heat` is `null`.
    pub heat: HeatGrid,
    /// Cells (indexes into `heat.values`) that the cost function's own `must` drops, whatever the
    /// goal's `must` says. Part of the formula, so only a server search changes it. Usually empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate: Vec<u32>,
}

/// One step of the funnel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// Plain words, e.g. "Points in the area", "Somewhere to stand", "On a roof".
    pub label: String,
    /// Candidates left after this stage.
    pub n: u32,
    /// The goal whose gate this stage is (`Goal::name()`); `None` for the fixed stages.
    #[serde(default)]
    pub goal: Option<String>,
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
    /// What the AI made of the spot after looking at it (photos, 3D); `None` if it didn't look.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub investigation: Option<Investigation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Investigation {
    pub verdict: Verdict,
    /// A few plain sentences: what it saw and what to do about it.
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The photos agree with the numbers.
    Confirmed,
    /// Usable, with a catch the note explains.
    Caution,
    /// Better (worse) than the numbers said; the AI moved it in `rank`.
    MovedUp,
    MovedDown,
    /// Doesn't work (blocked, no access...); kept so the user sees why it isn't higher.
    RuledOut,
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

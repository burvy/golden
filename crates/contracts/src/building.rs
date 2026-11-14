//! `Building`, `BuildingList` (docs/contracts.md#building-costar).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingList {
    /// Nearest first.
    pub buildings: Vec<Building>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Building {
    /// `costar:<property id>`.
    pub id: String,
    pub name: Option<String>,
    pub address: Option<String>,
    pub lat: f64,
    pub lon: f64,
    /// From the query point; None from `/api/buildings/{id}`.
    pub dist_m: Option<f64>,
    /// CoStar height above ground, metres.
    pub height_m: Option<f64>,
    pub floors: Option<u32>,
    /// Roof height from the DSM (same datum as all heights).
    pub roof_m: Option<f64>,
    pub property_type: Option<String>,
    pub contacts: Vec<Contact>,
    pub matterport: Vec<Matterport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    pub role: ContactRole,
    pub name: Option<String>,
    pub company: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactRole {
    Manager,
    Leasing,
    Owner,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matterport {
    pub id: String,
    pub url: String,
}

pub const FEET_TO_M: f64 = 0.3048;

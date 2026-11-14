//! `AgentRequest`, `CallSheet` (docs/contracts.md#agentrequest-and-callsheet): brief text in, call sheet out.

use serde::{Deserialize, Serialize};

use crate::building::Contact;
use crate::search::{default_tz, Area, Brief, Spot};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub text: String,
    #[serde(default)]
    pub area: Option<Area>,
    /// Local day; None = the agent picks.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default = "default_tz")]
    pub tz: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallSheet {
    pub title: String,
    /// The Brief the agent actually searched with.
    pub brief: Brief,
    pub summary: String,
    /// In time order.
    pub timeline: Vec<TimelineItem>,
    /// Exactly one primary, first; then backups.
    pub spots: Vec<SheetSpot>,
    pub access: Vec<Access>,
    pub caveats: Vec<String>,
    /// Model id, or "mock".
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineItem {
    pub t: String,
    pub what: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetSpot {
    pub role: SpotRole,
    pub spot: Spot,
    pub arrive_by: String,
    pub notes: Vec<String>,
    pub photo_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotRole {
    Primary,
    Backup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    pub building_id: String,
    pub building_name: Option<String>,
    pub contact: Option<Contact>,
    /// Ready-to-send one-line request.
    pub ask: String,
}

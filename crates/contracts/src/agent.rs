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
    /// What the agent did, in order, so the app can show its work. May be empty.
    #[serde(default)]
    pub steps: Vec<AgentStep>,
}

/// One thing the agent did: which tool (or "read" / "plan" / "write") and what came of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStep {
    pub tool: String,
    pub summary: String,
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

/// `POST /api/chat` request: the whole conversation so far (the server keeps no state).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    /// Oldest first. Starts with the user's first message; the agent's questions are sent back as `agent`.
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub area: Option<Area>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default = "default_tz")]
    pub tz: String,
    /// "Search now": skip any remaining questions and use the defaults.
    #[serde(default)]
    pub go: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Agent,
}

/// `POST /api/chat` answer: either a question (`ask`) or the finished search (`brief` + `sheet`). Exactly one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatReply {
    /// What the agent has understood so far, in plain words ("Sunset", "from a roof", "Princeton", ...).
    /// Once searching, the agent's own defaults end in " (my pick)" and goals the search can't score yet
    /// end in " (not checked yet)".
    pub understood: Vec<String>,
    #[serde(default)]
    pub ask: Option<ChatAsk>,
    /// The brief the chat settled on, with its `goals` (the sheet's brief may be relaxed or widened from it).
    #[serde(default)]
    pub brief: Option<Brief>,
    #[serde(default)]
    pub sheet: Option<CallSheet>,
    /// One plain sentence to show with the results, e.g. what the search couldn't check
    /// ("I can't place the fountain yet; searching for the sunset part."). Left out when there's nothing to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Who ran this chat turn: model id, or "mock".
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatAsk {
    pub question: String,
    /// Quick replies. The first is the default the agent would pick. May be empty.
    #[serde(default)]
    pub choices: Vec<String>,
}

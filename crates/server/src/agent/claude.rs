//! Claude (Anthropic Messages API, JSON output constrained by a schema). Used for the agent's two language
//! steps: reading the typed brief and writing the sheet's words. Every number still comes from our tools;
//! `times::times_ok` rejects text that mentions a time the tools didn't produce.
//!
//! Keys (environment or `env/.env`, never printed): `ANTHROPIC_API_KEY`, optional `GOLDEN_CLAUDE_MODEL`
//! (default `claude-sonnet-5-5`) and `ANTHROPIC_BASE_URL` (default `https://api.anthropic.com`).

use std::time::Duration;

use serde_json::{json, Value};

use crate::secrets;

/// Used when `GOLDEN_CLAUDE_MODEL` isn't set.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Both answers are a few hundred tokens; this leaves room for the model's (low-effort) thinking.
const MAX_TOKENS: u32 = 8000;
const TIMEOUT: Duration = Duration::from_secs(60);
/// Server-side refusal fallback (`fallbacks: "default"`), only sent to models that accept it.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const FALLBACK_MODELS: [&str; 4] = ["claude-sonnet-5-5", "claude-opus-5-5", "claude-opus-5", "claude-fable-5-1"];

#[derive(Clone)]
pub struct Claude {
    key: String,
    pub model: String,
    base_url: String,
    http: reqwest::Client,
}

impl Claude {
    pub fn from_env(http: &reqwest::Client) -> Option<Claude> {
        let key = secrets::get("ANTHROPIC_API_KEY")?;
        let model = secrets::get("GOLDEN_CLAUDE_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let base_url = secrets::get("ANTHROPIC_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Some(Claude::new(key, model, base_url, http))
    }

    pub fn new(key: String, model: String, base_url: String, http: &reqwest::Client) -> Claude {
        Claude { key, model, base_url: base_url.trim_end_matches('/').to_string(), http: http.clone() }
    }

    /// One prompt in, one JSON object out (matching `schema`, a JSON Schema with `additionalProperties: false`).
    pub async fn json(&self, prompt: &str, schema: Value) -> Result<Value, String> {
        let mut req = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .timeout(TIMEOUT)
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .json(&request_body(&self.model, prompt, schema));
        if uses_fallbacks(&self.model) {
            req = req.header("anthropic-beta", FALLBACK_BETA);
        }
        let resp = req.send().await.map_err(|e| format!("Claude request failed: {}", e.without_url()))?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().await.map_err(|e| format!("Claude answer unreadable: {}", e.without_url()))?;
        answer_json(status, &v)
    }
}

fn uses_fallbacks(model: &str) -> bool {
    FALLBACK_MODELS.contains(&model)
}

/// Pure: the Messages API request. Low effort: reading a brief and phrasing facts are easy, and the
/// app waits on both calls.
pub fn request_body(model: &str, prompt: &str, schema: Value) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "messages": [{ "role": "user", "content": prompt }],
        "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": schema } }
    });
    if uses_fallbacks(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

/// Pure: the JSON object in a Messages API answer, or why there isn't one. Thinking blocks come before the
/// text block, so take the first `text` block; a refusal or a cut-off answer may not match the schema.
pub fn answer_json(status: u16, v: &Value) -> Result<Value, String> {
    if !(200..300).contains(&status) {
        let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("Claude HTTP {status}: {}", msg.chars().take(200).collect::<String>()));
    }
    match v["stop_reason"].as_str() {
        Some("refusal") => return Err("Claude declined the request".into()),
        Some("max_tokens") => return Err("Claude's answer was cut off".into()),
        _ => {}
    }
    let text = v["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
        .and_then(|b| b["text"].as_str())
        .ok_or("Claude returned no text")?;
    serde_json::from_str(text).map_err(|e| format!("Claude JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_asks_for_schema_json_at_low_effort() {
        let b = request_body("claude-sonnet-5-5", "hi", json!({"type": "object"}));
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["output_config"]["format"]["schema"]["type"], "object");
        assert_eq!(b["output_config"]["effort"], "low");
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("thinking").is_none() && b.get("temperature").is_none()); // both 400 or are moot on current models
        assert!(request_body("claude-haiku-5-5", "hi", json!({})).get("fallbacks").is_none());
    }

    #[test]
    fn answer_takes_the_text_block_after_thinking() {
        let v = json!({"stop_reason": "end_turn", "content": [
            {"type": "thinking", "thinking": ""},
            {"type": "text", "text": "{\"a\": 1}"}
        ]});
        assert_eq!(answer_json(200, &v).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn answer_errors_are_reasons_not_panics() {
        let err = |s, v| answer_json(s, &v).unwrap_err();
        assert!(err(401, json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}})).contains("401"));
        assert!(err(200, json!({"stop_reason": "refusal", "content": []})).contains("declined"));
        assert!(err(200, json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"a\""}]})).contains("cut off"));
        assert!(err(200, json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "not json"}]})).contains("JSON"));
        assert!(err(200, json!({"stop_reason": "end_turn", "content": []})).contains("no text"));
    }
}

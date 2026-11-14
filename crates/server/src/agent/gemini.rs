//! Gemini (REST `generateContent`, JSON output with a schema). Used for the two language steps of the agent:
//! reading the typed brief and writing the sheet's words. Every number still comes from our tools; `times_ok`
//! rejects text that mentions a time the tools didn't produce. Keys from `env/.env`: `GEMINI_API_KEY`,
//! optional `GEMINI_MODEL`.

use serde_json::{json, Value};

use crate::secrets;

/// Used when `GEMINI_MODEL` isn't set: Google's alias for its current Flash model.
pub const DEFAULT_MODEL: &str = "gemini-flash-latest";

#[derive(Clone)]
pub struct Gemini {
    key: String,
    pub model: String,
    http: reqwest::Client,
}

impl Gemini {
    pub fn from_env(http: &reqwest::Client) -> Option<Gemini> {
        let key = secrets::get("GEMINI_API_KEY")?;
        let model = secrets::get("GEMINI_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        Some(Gemini { key, model, http: http.clone() })
    }

    /// One prompt in, one JSON object out (matching `schema`, an OpenAPI-style schema).
    pub async fn json(&self, prompt: &str, schema: Value) -> Result<Value, String> {
        let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", self.model);
        let body = json!({
            "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
            "generationConfig": { "responseMimeType": "application/json", "responseSchema": schema, "temperature": 0.2 }
        });
        let resp = self
            .http
            .post(url)
            .header("x-goog-api-key", &self.key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Gemini request failed: {}", e.without_url()))?;
        let status = resp.status();
        let v: Value = resp.json().await.map_err(|e| format!("Gemini answer unreadable: {}", e.without_url()))?;
        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
            return Err(format!("Gemini HTTP {}: {}", status.as_u16(), msg.chars().take(200).collect::<String>()));
        }
        let text = v["candidates"][0]["content"]["parts"][0]["text"].as_str().ok_or("Gemini returned no text")?;
        serde_json::from_str(text).map_err(|e| format!("Gemini JSON: {e}"))
    }
}

/// Every clock time ("19:58", "7:58") in `text` must be one of `allowed` (as "HH:MM"). The model may only
/// repeat times our tools computed.
pub fn times_ok(text: &str, allowed: &[String]) -> bool {
    clock_times(text).iter().all(|t| allowed.iter().any(|a| a == t || twelve_hour(a).as_deref() == Some(t.as_str())))
}

/// "19:58" -> "7:58"
fn twelve_hour(hhmm: &str) -> Option<String> {
    let (h, m) = hhmm.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    Some(format!("{}:{m}", if h % 12 == 0 { 12 } else { h % 12 }))
}

/// Every `H:MM` / `HH:MM` in the text, as written. Not "16:9" (one digit after) or "123:45" (three before).
pub fn clock_times(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(|c| c.is_ascii_digit());
    let mut out = Vec::new();
    for i in 1..b.len() {
        if b[i] != b':' || !digit(i + 1) || !digit(i + 2) || digit(i + 3) {
            continue;
        }
        let before = (1..=3).take_while(|k| i >= *k && digit(i - k)).count();
        if before == 1 || before == 2 {
            out.push(text[i - before..i + 3].to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_clock_times() {
        assert_eq!(clock_times("Sun enters the gap at 19:58, arrive 7:15 pm"), vec!["19:58", "7:15"]);
        assert!(clock_times("ratio 16:9 and 123:45 and 3:5").is_empty());
        assert!(clock_times("no times here").is_empty());
    }

    #[test]
    fn only_tool_times_pass() {
        let allowed = vec!["19:58".to_string(), "19:15".to_string()];
        assert!(times_ok("The sun drops into the gap at 19:58; arrive by 7:15.", &allowed));
        assert!(!times_ok("The sun sets at 20:07.", &allowed));
        assert!(times_ok("No numbers at all.", &allowed));
    }
}

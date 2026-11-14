//! The Vexcel API key. Comes from `VEXCEL_API_KEY`, or from the file named by `VEXCEL_ENV_FILE`
//! (a `.env` style file), or from `env/.env` (the documented, gitignored place) when neither is set. It is never printed: `Debug` is redacted and there is no `Display`.

use crate::Error;

/// Where the key lives if no variable says otherwise (relative to where the server runs: the repo root).
pub const DEFAULT_ENV_FILE: &str = "env/.env";

#[derive(Clone)]
pub struct ApiKey(String);

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

impl ApiKey {
    pub fn new(s: impl Into<String>) -> ApiKey {
        ApiKey(s.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// `VEXCEL_API_KEY` wins; otherwise read it from the file in `VEXCEL_ENV_FILE`, else `env/.env`.
    pub fn from_env() -> Result<ApiKey, Error> {
        if let Ok(k) = std::env::var("VEXCEL_API_KEY") {
            if !k.trim().is_empty() {
                return Ok(ApiKey(k.trim().to_string()));
            }
        }
        let path = std::env::var("VEXCEL_ENV_FILE").unwrap_or_else(|_| DEFAULT_ENV_FILE.to_string());
        let text = std::fs::read_to_string(&path).map_err(|_| Error::NoKey)?;
        find_key(&text).map(ApiKey).ok_or(Error::NoKey)
    }
}

/// Find `VEXCEL_API_KEY=...` in `.env` text. Handles `export`, quotes, comments and CRLF.
pub fn find_key(env_text: &str) -> Option<String> {
    env_text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let value = line.strip_prefix("VEXCEL_API_KEY")?.trim_start().strip_prefix('=')?;
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_key_in_env_text() {
        assert_eq!(find_key("# hi\r\nOTHER=1\r\nVEXCEL_API_KEY=abc123\r\n").as_deref(), Some("abc123"));
        assert_eq!(find_key("export VEXCEL_API_KEY = \"q w\"").as_deref(), Some("q w"));
        assert_eq!(find_key("VEXCEL_API_KEY_OLD=nope"), None);
        assert_eq!(find_key("VEXCEL_API_KEY="), None);
        assert_eq!(find_key("# VEXCEL_API_KEY=commented"), None);
    }

    #[test]
    fn debug_never_shows_key() {
        let k = ApiKey::new("supersecret");
        assert!(!format!("{k:?}").contains("supersecret"));
    }
}

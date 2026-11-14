//! Keys from the environment or the gitignored env file (`env/.env`, or the path in `GOLDEN_ENV_FILE` /
//! `VEXCEL_ENV_FILE`). Values are never printed or logged.

use std::path::PathBuf;

fn env_file() -> PathBuf {
    std::env::var_os("GOLDEN_ENV_FILE")
        .or_else(|| std::env::var_os("VEXCEL_ENV_FILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("env/.env"))
}

/// `name` from the process environment, else from the env file. Empty values count as missing.
pub fn get(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    let text = std::fs::read_to_string(env_file()).ok()?;
    parse(&text, name)
}

/// Pure: the value of `name` in `KEY=value` lines (quotes and spaces trimmed, `#` comments skipped).
pub fn parse(text: &str, name: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("export ").unwrap_or(l).split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_key_value_lines() {
        let t = "# keys\nVEXCEL_API_KEY=abc\nexport ANTHROPIC_API_KEY=\"a-1\"\nEMPTY=\n";
        assert_eq!(parse(t, "ANTHROPIC_API_KEY").as_deref(), Some("a-1"));
        assert_eq!(parse(t, "VEXCEL_API_KEY").as_deref(), Some("abc"));
        assert_eq!(parse(t, "EMPTY"), None);
        assert_eq!(parse(t, "MISSING"), None);
    }
}

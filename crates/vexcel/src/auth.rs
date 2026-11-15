//! API keys, and the one reader the whole workspace uses for them (the server's `secrets` calls it too).
//!
//! A key `NAME` (e.g. `VEXCEL_API_KEY`) is looked up in this order:
//! 1. the process environment variable `NAME`;
//! 2. its own key file, if the variable naming it is set (`VEXCEL_ENV_FILE`, `ANTHROPIC_ENV_FILE`). That file may
//!    be a `.env` with `NAME=value`, or just the bare token on one line (which is how keys are usually handed out);
//! 3. the shared `.env` file: `GOLDEN_ENV_FILE`, else `VEXCEL_ENV_FILE`, else `env/.env`. Only `NAME=value` counts
//!    there, so one key's bare file is never read as another key.
//!
//! Keys are never printed: `ApiKey`'s `Debug` is redacted and there is no `Display`.

use std::path::Path;

use crate::Error;

/// Where the keys live if no variable says otherwise (relative to where the server runs: the repo root).
pub const DEFAULT_ENV_FILE: &str = "env/.env";
/// The variable naming the shared `.env` file (checked before `VEXCEL_ENV_FILE` and `env/.env`).
pub const SHARED_ENV_FILE_VAR: &str = "GOLDEN_ENV_FILE";
/// The Vexcel key, and the variable naming its own key file.
pub const VEXCEL_KEY: &str = "VEXCEL_API_KEY";
pub const VEXCEL_KEY_FILE_VAR: &str = "VEXCEL_ENV_FILE";
/// Set to `1` for cache-only mode: `ApiKey::from_env` then always says `NoKey`, so every Vexcel call in the
/// workspace (heights, photos, tiles, mesh, detection) serves what is cached and never downloads.
pub const OFFLINE_VAR: &str = "GOLDEN_OFFLINE";

/// Is cache-only mode on (`GOLDEN_OFFLINE` = 1/true/yes/on)?
pub fn offline() -> bool {
    std::env::var(OFFLINE_VAR).is_ok_and(|v| is_on(&v))
}

/// Pure: does an env value switch something on?
pub fn is_on(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

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

    /// `VEXCEL_API_KEY`, else the file in `VEXCEL_ENV_FILE` (bare token or `.env`), else the shared `.env`.
    /// Always `NoKey` in cache-only mode (`GOLDEN_OFFLINE=1`).
    pub fn from_env() -> Result<ApiKey, Error> {
        if offline() {
            return Err(Error::NoKey);
        }
        lookup(VEXCEL_KEY, Some(VEXCEL_KEY_FILE_VAR)).map(ApiKey).ok_or(Error::NoKey)
    }
}

/// The key `name` from the real environment and files (see the module docs for the order).
pub fn lookup(name: &str, own_file_var: Option<&str>) -> Option<String> {
    lookup_with(name, own_file_var, |v| std::env::var(v).ok(), |p| std::fs::read_to_string(p).ok())
}

/// `lookup` with the environment and the file reads passed in, so the order can be tested without touching either.
pub fn lookup_with(
    name: &str,
    own_file_var: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<String> {
    let set = |v: &str| env(v).filter(|s| !s.trim().is_empty());
    if let Some(k) = set(name) {
        return Some(k.trim().to_string());
    }
    let own = own_file_var.and_then(set);
    if let Some(k) = own.as_deref().and_then(|p| read(Path::new(p.trim()))).and_then(|t| find_key(&t, name, true)) {
        return Some(k);
    }
    let shared = set(SHARED_ENV_FILE_VAR).or_else(|| set(VEXCEL_KEY_FILE_VAR)).unwrap_or_else(|| DEFAULT_ENV_FILE.into());
    read(Path::new(shared.trim())).and_then(|t| find_key(&t, name, false))
}

/// Pure: the key `name` in a key file's text. `NAME=value` lines always count (`export`, quotes, `#` comments and
/// CRLF handled). With `bare_ok`, a file that is just one token is that key: after skipping blank and `#` lines there
/// must be exactly one line, with no spaces, that isn't some other `KEY=value`.
pub fn find_key(text: &str, name: &str, bare_ok: bool) -> Option<String> {
    assigned(text, name).or_else(|| bare_ok.then(|| bare_token(text)).flatten())
}

/// The value of `name=...` in `.env` text, if there is a non-empty one.
fn assigned(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let value = line.strip_prefix(name)?.trim_start().strip_prefix('=')?;
        let value = unquote(value.trim());
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The text's only meaningful line, if it reads as a token rather than a `.env` line or prose.
fn bare_token(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let token = unquote(line);
    let ok = !token.is_empty() && !token.contains(char::is_whitespace) && !is_assignment(token);
    ok.then(|| token.to_string())
}

/// `IDENT=...` (a `.env` line). A token ending in base64 padding (`abc==`) is not one, but `IDENT=` with nothing
/// after it is: so a key whose only `=` is a single trailing one has to be written as `NAME=key`.
fn is_assignment(line: &str) -> bool {
    let Some((key, rest)) = line.split_once('=') else { return false };
    let ident = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let padding = !rest.is_empty() && rest.chars().all(|c| c == '=');
    ident && !padding
}

fn unquote(s: &str) -> &str {
    s.trim_matches(|c| c == '"' || c == '\'')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const K: &str = "VEXCEL_API_KEY";

    #[test]
    fn finds_name_value_lines() {
        assert_eq!(find_key("# hi\r\nOTHER=1\r\nVEXCEL_API_KEY=abc123\r\n", K, false).as_deref(), Some("abc123"));
        assert_eq!(find_key("export VEXCEL_API_KEY = \"q w\"", K, false).as_deref(), Some("q w"));
        assert_eq!(find_key("VEXCEL_API_KEY=abc123", K, true).as_deref(), Some("abc123"));
        assert_eq!(find_key("VEXCEL_API_KEY_OLD=nope", K, false), None);
        assert_eq!(find_key("VEXCEL_API_KEY=", K, false), None);
        assert_eq!(find_key("# VEXCEL_API_KEY=commented", K, false), None);
    }

    #[test]
    fn accepts_a_bare_token_file() {
        for text in ["tok-123", "tok-123\n", "tok-123\r\n", "\n  tok-123  \r\n\r\n", "# my key\ntok-123\n", "\"tok-123\""] {
            assert_eq!(find_key(text, K, true).as_deref(), Some("tok-123"), "{text:?}");
        }
        assert_eq!(find_key("dGVzdA==\n", K, true).as_deref(), Some("dGVzdA=="), "base64 padding is a token");
    }

    #[test]
    fn bare_tokens_only_where_allowed() {
        assert_eq!(find_key("tok-123\n", K, false), None, "a shared .env never yields a bare token");
    }

    #[test]
    fn rejects_files_that_are_not_one_token() {
        for text in [
            "",
            "\r\n# only a comment\r\n",
            "tok-1\ntok-2\n",
            "tok-1\r\ntok-2",
            "OTHER_KEY=abc\n",
            "ANTHROPIC_API_KEY=sk-1\n",
            "FOO=\n",
            "export OTHER=1",
            "this is not a key",
            "tok-1\nOTHER=2\n",
        ] {
            assert_eq!(find_key(text, K, true), None, "{text:?}");
        }
    }

    /// Runs `lookup_with` against a fake environment and fake files.
    fn look(name: &str, own: Option<&str>, env: &[(&str, &str)], files: &[(&str, &str)]) -> Option<String> {
        let env: HashMap<String, String> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let files: HashMap<String, String> = files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        lookup_with(name, own, |v| env.get(v).cloned(), |p| files.get(p.to_str().unwrap()).cloned())
    }

    #[test]
    fn lookup_order() {
        let own = Some("VEXCEL_ENV_FILE");
        let vfile = [("VEXCEL_ENV_FILE", "v.env")];
        // The variable beats every file.
        assert_eq!(look(K, own, &[(K, " direct \n"), vfile[0]], &[("v.env", "from-file")]).as_deref(), Some("direct"));
        // Bare token in its own file.
        assert_eq!(look(K, own, &vfile, &[("v.env", "from-file\r\n")]).as_deref(), Some("from-file"));
        // No variables: the default env/.env, NAME=value only.
        assert_eq!(look(K, own, &[], &[("env/.env", "VEXCEL_API_KEY=dflt\n")]).as_deref(), Some("dflt"));
        assert_eq!(look(K, own, &[], &[("env/.env", "bare\n")]), None);
        // An empty variable counts as unset.
        assert_eq!(look(K, own, &[(K, "  ")], &[("env/.env", "VEXCEL_API_KEY=dflt")]).as_deref(), Some("dflt"));
        // Nothing anywhere.
        assert_eq!(look(K, own, &vfile, &[]), None);
    }

    #[test]
    fn one_keys_bare_file_is_never_another_key() {
        let env = [("VEXCEL_ENV_FILE", "v.env"), ("ANTHROPIC_ENV_FILE", "c.env")];
        let files = [("v.env", "vexcel-token\n"), ("c.env", "claude-token\n")];
        assert_eq!(look(K, Some("VEXCEL_ENV_FILE"), &env, &files).as_deref(), Some("vexcel-token"));
        assert_eq!(look("ANTHROPIC_API_KEY", Some("ANTHROPIC_ENV_FILE"), &env, &files).as_deref(), Some("claude-token"));
        // Without its own file, the Claude key is not taken from the Vexcel one (the shared fallback), nor is anything else.
        assert_eq!(look("ANTHROPIC_API_KEY", Some("ANTHROPIC_ENV_FILE"), &env[..1], &files), None);
        assert_eq!(look("GOOGLE_MAPS_API_KEY", None, &env, &files), None);
    }

    #[test]
    fn shared_env_file_still_holds_several_keys() {
        let text = "VEXCEL_API_KEY=v\nANTHROPIC_API_KEY=\"a-1\"\n";
        let golden = [("GOLDEN_ENV_FILE", "g.env")];
        assert_eq!(look("ANTHROPIC_API_KEY", Some("ANTHROPIC_ENV_FILE"), &golden, &[("g.env", text)]).as_deref(), Some("a-1"));
        assert_eq!(look(K, Some("VEXCEL_ENV_FILE"), &golden, &[("g.env", text)]).as_deref(), Some("v"));
        // The old way: VEXCEL_ENV_FILE pointing at a multi-key .env still serves the others.
        let vexcel = [("VEXCEL_ENV_FILE", "v.env")];
        assert_eq!(look("ANTHROPIC_API_KEY", Some("ANTHROPIC_ENV_FILE"), &vexcel, &[("v.env", text)]).as_deref(), Some("a-1"));
        // GOLDEN_ENV_FILE wins over VEXCEL_ENV_FILE as the shared file.
        let both = [("GOLDEN_ENV_FILE", "g.env"), ("VEXCEL_ENV_FILE", "v.env")];
        let files = [("g.env", "ANTHROPIC_API_KEY=g"), ("v.env", "ANTHROPIC_API_KEY=v")];
        assert_eq!(look("ANTHROPIC_API_KEY", None, &both, &files).as_deref(), Some("g"));
    }

    #[test]
    fn debug_never_shows_key() {
        let k = ApiKey::new("supersecret");
        assert!(!format!("{k:?}").contains("supersecret"));
    }
}

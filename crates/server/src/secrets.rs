//! Keys and settings from the environment or the gitignored key files. One reader for the whole workspace:
//! `vexcel::auth::lookup` (see its docs for the order). Values are never printed or logged.

/// The variable naming a key's own file, which may hold just the bare token.
fn own_file_var(name: &str) -> Option<&'static str> {
    match name {
        "ANTHROPIC_API_KEY" => Some("ANTHROPIC_ENV_FILE"),
        vexcel::auth::VEXCEL_KEY => Some(vexcel::auth::VEXCEL_KEY_FILE_VAR),
        _ => None,
    }
}

/// `name` from the process environment, else its own key file, else the shared `.env`. Empty values count as missing.
pub fn get(name: &str) -> Option<String> {
    vexcel::auth::lookup(name, own_file_var(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_keys_have_their_own_file() {
        assert_eq!(own_file_var("ANTHROPIC_API_KEY"), Some("ANTHROPIC_ENV_FILE"));
        assert_eq!(own_file_var("VEXCEL_API_KEY"), Some("VEXCEL_ENV_FILE"));
        // Settings like the model name are never read from a bare-token file.
        assert_eq!(own_file_var("GOLDEN_CLAUDE_MODEL"), None);
        assert_eq!(own_file_var("ANTHROPIC_BASE_URL"), None);
    }
}

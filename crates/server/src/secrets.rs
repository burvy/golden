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
/// With `GOLDEN_OFFLINE=1` the data keys (Vexcel, Google) are hidden, so nothing downloads (`guard.rs`).
pub fn get(name: &str) -> Option<String> {
    if crate::guard::get().offline && hidden_offline(name) {
        return None;
    }
    vexcel::auth::lookup(name, own_file_var(name))
}

/// Keys that only fetch data, so cache-only mode drops them. Claude has its own switch (`GOLDEN_CLAUDE`).
fn hidden_offline(name: &str) -> bool {
    matches!(name, "GOOGLE_MAPS_API_KEY" | vexcel::auth::VEXCEL_KEY)
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

    #[test]
    fn offline_hides_the_data_keys_only() {
        assert!(hidden_offline("GOOGLE_MAPS_API_KEY") && hidden_offline("VEXCEL_API_KEY"));
        assert!(!hidden_offline("ANTHROPIC_API_KEY"));
    }
}

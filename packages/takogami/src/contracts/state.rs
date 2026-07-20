//! Operational session state-home resolution.
//!
//! Precedence: `--state-home` → `TAKOGAMI_STATE_HOME` → profile `[runtime] session_state_home`
//! → `$XDG_STATE_HOME/takogami/sessions` → `~/.local/state/takogami/sessions`.
//!
//! `logs.session_log_target` is tracked build-session provenance and must never be used here.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct StateHomeInputs<'a> {
    pub cli_state_home: Option<&'a Path>,
    pub env_takogami_state_home: Option<&'a str>,
    pub profile_session_state_home: Option<&'a str>,
    pub env_xdg_state_home: Option<&'a str>,
    pub home_dir: Option<&'a Path>,
}

/// Resolve the operational runtime-session directory.
pub fn resolve_session_state_home(inputs: StateHomeInputs<'_>) -> PathBuf {
    if let Some(path) = inputs.cli_state_home {
        return path.to_path_buf();
    }
    if let Some(path) = inputs.env_takogami_state_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = inputs.profile_session_state_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(xdg) = inputs.env_xdg_state_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(xdg).join("takogami").join("sessions");
    }
    let home = inputs
        .home_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    home.join(".local")
        .join("state")
        .join("takogami")
        .join("sessions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn cli_wins() {
        let cli = Path::new("/tmp/cli-state");
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: Some(cli),
            env_takogami_state_home: Some("/tmp/env"),
            profile_session_state_home: Some("/tmp/profile"),
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, cli);
    }

    #[test]
    fn env_before_profile() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: Some("/tmp/env"),
            profile_session_state_home: Some("/tmp/profile"),
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, PathBuf::from("/tmp/env"));
    }

    #[test]
    fn profile_before_xdg() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: None,
            profile_session_state_home: Some("/tmp/profile"),
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, PathBuf::from("/tmp/profile"));
    }

    #[test]
    fn xdg_before_home_fallback() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: None,
            profile_session_state_home: None,
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, PathBuf::from("/tmp/xdg/takogami/sessions"));
    }

    #[test]
    fn home_fallback() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: None,
            profile_session_state_home: None,
            env_xdg_state_home: None,
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(
            got,
            PathBuf::from("/Users/x/.local/state/takogami/sessions")
        );
    }
}

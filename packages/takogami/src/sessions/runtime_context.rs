//! Bounded Herdr/tmux opaque runtime-context collection.

use crate::contracts::types::{DiagnosticRecord, RuntimeContext};

const MAX_ID_BYTES: usize = 128;

#[derive(Debug, Default)]
pub struct RuntimeContextEnv<'a> {
    pub herdr_workspace_id: Option<&'a str>,
    pub herdr_tab_id: Option<&'a str>,
    pub herdr_pane_id: Option<&'a str>,
    pub tmux: Option<&'a str>,
    pub tmux_pane: Option<&'a str>,
}

/// Collect opaque runtime context. Invalid optional context yields a diagnostic and omission.
pub fn collect_runtime_context(
    env: RuntimeContextEnv<'_>,
) -> (Option<RuntimeContext>, Option<DiagnosticRecord>) {
    let herdr_any = env.herdr_workspace_id.is_some()
        || env.herdr_tab_id.is_some()
        || env.herdr_pane_id.is_some();
    if herdr_any {
        return match (
            normalize_id(env.herdr_workspace_id),
            normalize_id(env.herdr_tab_id),
            normalize_id(env.herdr_pane_id),
        ) {
            (Ok(workspace_id), Ok(tab_id), Ok(pane_id)) => (
                Some(RuntimeContext {
                    provider: "herdr".into(),
                    workspace_id,
                    tab_id,
                    pane_id,
                }),
                None,
            ),
            _ => (
                None,
                Some(DiagnosticRecord {
                    code: "runtime_context_invalid".into(),
                    message: "herdr runtime context omitted: invalid opaque id".into(),
                }),
            ),
        };
    }

    if env.tmux.filter(|s| !s.is_empty()).is_some() {
        return match normalize_id(env.tmux_pane) {
            Ok(Some(pane_id)) => (
                Some(RuntimeContext {
                    provider: "tmux".into(),
                    workspace_id: None,
                    tab_id: None,
                    pane_id: Some(pane_id),
                }),
                None,
            ),
            Ok(None) => (None, None),
            Err(()) => (
                None,
                Some(DiagnosticRecord {
                    code: "runtime_context_invalid".into(),
                    message: "tmux runtime context omitted: invalid opaque id".into(),
                }),
            ),
        };
    }

    (None, None)
}

fn normalize_id(raw: Option<&str>) -> Result<Option<String>, ()> {
    let Some(value) = raw.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if value.len() > MAX_ID_BYTES {
        return Err(());
    }
    if value.contains('/') || value.contains('\\') || value.contains('\0') {
        return Err(());
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(());
    }
    Ok(Some(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herdr_wins_over_tmux() {
        let (ctx, diag) = collect_runtime_context(RuntimeContextEnv {
            herdr_workspace_id: Some("w1"),
            herdr_tab_id: Some("w1:t1"),
            herdr_pane_id: Some("w1:p2"),
            tmux: Some("/tmp/tmux-1000/default"),
            tmux_pane: Some("%3"),
        });
        assert!(diag.is_none());
        let ctx = ctx.unwrap();
        assert_eq!(ctx.provider, "herdr");
        assert_eq!(ctx.pane_id.as_deref(), Some("w1:p2"));
    }

    #[test]
    fn tmux_socket_never_persisted() {
        let (ctx, _) = collect_runtime_context(RuntimeContextEnv {
            herdr_workspace_id: None,
            herdr_tab_id: None,
            herdr_pane_id: None,
            tmux: Some("/tmp/tmux-1000/default"),
            tmux_pane: Some("%3"),
        });
        let ctx = ctx.unwrap();
        let text = serde_json::to_string(&ctx).unwrap();
        assert!(!text.contains("/tmp/tmux"));
        assert_eq!(ctx.pane_id.as_deref(), Some("%3"));
    }
}

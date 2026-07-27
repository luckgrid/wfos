//! D9 Option B validated bin cleanup scopes.

use std::path::{Component, Path};

/// Workspace-relative cleanup scope: `namespace/bin/<segment>[/<segment>...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedBinScope {
    normalized: String,
}

impl ValidatedBinScope {
    /// Validate and normalize an explicit `--scope` operand before policy or spawn.
    pub(crate) fn parse(raw: &str) -> Result<Self, ScopeError> {
        if raw.is_empty() || raw == "." || raw == ".." || raw == "~" {
            return Err(ScopeError::Invalid);
        }
        if raw.contains('\\') || raw.chars().any(|c| c.is_control()) {
            return Err(ScopeError::Invalid);
        }
        if Path::new(raw).is_absolute() || raw.starts_with('/') || raw.starts_with('~') {
            return Err(ScopeError::Invalid);
        }

        let path = Path::new(raw);
        let mut segments = Vec::new();
        for comp in path.components() {
            match comp {
                Component::Normal(s) => {
                    let Some(text) = s.to_str() else {
                        return Err(ScopeError::Invalid);
                    };
                    if text.is_empty() || text == "." || text == ".." {
                        return Err(ScopeError::Invalid);
                    }
                    if !is_segment(text) {
                        return Err(ScopeError::Invalid);
                    }
                    segments.push(text);
                }
                Component::CurDir
                | Component::ParentDir
                | Component::RootDir
                | Component::Prefix(_) => {
                    return Err(ScopeError::Invalid);
                }
            }
        }

        // namespace/bin/segment+
        if segments.len() < 3 || segments[1] != "bin" {
            return Err(ScopeError::Invalid);
        }
        let ns = segments[0];
        if ns == "lib" || ns == "src" {
            return Err(ScopeError::Invalid);
        }

        let normalized = segments.join("/");
        // Phase 1 Ontarch grammar (defense in depth; do not widen schema).
        if !matches_phase1_grammar(&normalized) {
            return Err(ScopeError::Invalid);
        }
        Ok(Self { normalized })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.normalized
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeError {
    Invalid,
}

impl ScopeError {
    pub(crate) fn code(self) -> &'static str {
        "bin_scope_invalid"
    }

    pub(crate) fn message(self) -> &'static str {
        "invalid bin cleanup --scope (require namespace/bin/<segment>+; reject absolute, traversal, namespace roots, lib/src)"
    }
}

fn is_segment(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

fn matches_phase1_grammar(path: &str) -> bool {
    // ^[A-Za-z0-9][A-Za-z0-9._-]*/bin(/[A-Za-z0-9][A-Za-z0-9._-]*)+$
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 3 || parts[1] != "bin" {
        return false;
    }
    parts.iter().all(|seg| is_segment(seg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_workflow_and_subtree() {
        assert_eq!(
            ValidatedBinScope::parse("Build/bin/wfos").unwrap().as_str(),
            "Build/bin/wfos"
        );
        assert_eq!(
            ValidatedBinScope::parse("Build/bin/wfos/reviews")
                .unwrap()
                .as_str(),
            "Build/bin/wfos/reviews"
        );
        assert_eq!(
            ValidatedBinScope::parse("Plan/bin/research")
                .unwrap()
                .as_str(),
            "Plan/bin/research"
        );
    }

    #[test]
    fn rejects_namespace_roots_and_invalid() {
        for bad in [
            "Plan/bin",
            "Build/bin",
            "Control/bin",
            "/bin/wfos",
            "../Build/bin/wfos",
            "Build/bin/../lib",
            "Build\\bin\\wfos",
            ".",
            "~",
            "lib/secret",
            "src/bin/demo",
            "Build/bin",
            "",
            "Build/bin/\u{0001}x",
        ] {
            assert!(
                ValidatedBinScope::parse(bad).is_err(),
                "expected reject for {bad:?}"
            );
        }
    }
}

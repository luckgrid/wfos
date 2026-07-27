//! Exact-key environment snapshot for authorized children (never log values).

use std::collections::BTreeMap;

use crate::contracts::types::DiagnosticRecord;

const SECRET_INDICATORS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "PRIVATE_KEY",
    "ACCESS_KEY",
    "AUTH",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvError {
    /// Key name looks secret-bearing; values are never included in the message.
    SecretKeyName { key: String },
    /// Fixed and inherited sets conflict or contain duplicates.
    Conflict { key: String },
}

impl EnvError {
    pub fn diagnostic(&self) -> DiagnosticRecord {
        match self {
            Self::SecretKeyName { key } => DiagnosticRecord {
                code: "execution_contract".into(),
                message: format!("refusing secret-named environment key `{key}`"),
            },
            Self::Conflict { key } => DiagnosticRecord {
                code: "execution_contract".into(),
                message: format!("environment key conflict for `{key}`"),
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvSnapshot {
    pub pairs: Vec<(String, String)>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

/// True when a key name contains a case-insensitive secret indicator.
pub fn is_secret_key_name(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    SECRET_INDICATORS
        .iter()
        .any(|needle| upper.contains(needle))
}

/// Snapshot only sealed key names from the current process environment.
///
/// Missing keys are omitted with key-name-only diagnostics. Values are never logged.
pub fn snapshot_env(keys: &[String]) -> Result<EnvSnapshot, EnvError> {
    let mut pairs = Vec::new();
    let mut diagnostics = Vec::new();
    for key in keys {
        if is_secret_key_name(key) {
            return Err(EnvError::SecretKeyName { key: key.clone() });
        }
        match std::env::var(key) {
            Ok(value) => pairs.push((key.clone(), value)),
            Err(std::env::VarError::NotPresent) => {
                diagnostics.push(DiagnosticRecord {
                    code: "env_key_missing".into(),
                    message: format!("declared environment key `{key}` is not set"),
                });
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                diagnostics.push(DiagnosticRecord {
                    code: "env_key_invalid".into(),
                    message: format!("declared environment key `{key}` is not valid Unicode"),
                });
            }
        }
    }
    Ok(EnvSnapshot { pairs, diagnostics })
}

/// Validate a controller-built PATH: absolute directories only, no empty/relative components.
pub fn validate_controller_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("PATH must not be empty".into());
    }
    let mut seen = BTreeMap::new();
    for part in path.split(':') {
        if part.is_empty() {
            return Err("PATH must not contain empty components".into());
        }
        let p = std::path::Path::new(part);
        if !p.is_absolute() {
            return Err("PATH components must be absolute".into());
        }
        if part.contains("/./")
            || part.contains("/../")
            || part.ends_with("/.")
            || part.ends_with("/..")
            || part == "."
            || part == ".."
        {
            return Err("PATH must not contain relative components".into());
        }
        if seen.insert(part.to_string(), ()).is_some() {
            return Err("PATH must not contain duplicate directories".into());
        }
    }
    Ok(())
}

/// Build a child environment from controller-fixed pairs plus approved inherited keys.
///
/// Fixed values always win. Caller values never override fixed keys. Secret-like inherited
/// names are refused. Fixed/inherited key-name conflicts are refused.
pub fn build_child_env(
    fixed: &BTreeMap<String, String>,
    inherited_keys: &[String],
) -> Result<EnvSnapshot, EnvError> {
    let mut seen_fixed = BTreeMap::new();
    for (k, v) in fixed {
        if seen_fixed.insert(k.clone(), ()).is_some() {
            return Err(EnvError::Conflict { key: k.clone() });
        }
        if is_secret_key_name(k) {
            // Fixed controller keys like PANOPLY_AGENT are allowed by explicit allowlist.
            if k != "PANOPLY_AGENT" && k != "NO_COLOR" && k != "WS_ROOT" && k != "PATH" {
                return Err(EnvError::SecretKeyName { key: k.clone() });
            }
        }
        if k == "PATH"
            && let Err(msg) = validate_controller_path(v)
        {
            return Err(EnvError::Conflict {
                key: format!("PATH ({msg})"),
            });
        }
        let _ = v;
    }

    let mut inherited_seen = BTreeMap::new();
    for key in inherited_keys {
        if fixed.contains_key(key) {
            return Err(EnvError::Conflict { key: key.clone() });
        }
        if inherited_seen.insert(key.clone(), ()).is_some() {
            return Err(EnvError::Conflict { key: key.clone() });
        }
        if is_secret_key_name(key) {
            return Err(EnvError::SecretKeyName { key: key.clone() });
        }
    }

    let inherited = snapshot_env(inherited_keys)?;
    let mut pairs: Vec<(String, String)> =
        fixed.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    pairs.extend(inherited.pairs);
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(EnvSnapshot {
        pairs,
        diagnostics: inherited.diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_name_guard_is_case_insensitive() {
        assert!(is_secret_key_name("API_TOKEN"));
        assert!(is_secret_key_name("db_password"));
        assert!(is_secret_key_name("ssh_auth_sock"));
        assert!(!is_secret_key_name("PATH"));
        assert!(!is_secret_key_name("HOME"));
    }

    #[test]
    fn missing_keys_are_omitted_with_name_only_diagnostics() {
        let snap = snapshot_env(&["TAKOGAMI_TEST_ENV_MISSING_XYZ_99".into(), "PATH".into()])
            .expect("non-secret keys");
        assert!(
            snap.pairs
                .iter()
                .all(|(k, _)| k != "TAKOGAMI_TEST_ENV_MISSING_XYZ_99")
        );
        assert!(
            snap.diagnostics
                .iter()
                .any(|d| d.message.contains("TAKOGAMI_TEST_ENV_MISSING_XYZ_99")
                    && !d.message.contains('='))
        );
        let _ = snap.pairs.iter().find(|(k, _)| k == "PATH");
    }

    #[test]
    fn fixed_panoply_agent_cannot_be_inherited_conflict() {
        let mut fixed = BTreeMap::new();
        fixed.insert("PANOPLY_AGENT".into(), "1".into());
        let err = build_child_env(&fixed, &["PANOPLY_AGENT".into()]).unwrap_err();
        assert!(matches!(err, EnvError::Conflict { .. }));
    }

    #[test]
    fn fixed_overrides_caller_completely() {
        let mut fixed = BTreeMap::new();
        fixed.insert("PANOPLY_AGENT".into(), "1".into());
        fixed.insert("NO_COLOR".into(), "1".into());
        let snap = build_child_env(&fixed, &[]).unwrap();
        assert_eq!(
            snap.pairs
                .iter()
                .find(|(k, _)| k == "PANOPLY_AGENT")
                .map(|(_, v)| v.as_str()),
            Some("1")
        );
    }
}

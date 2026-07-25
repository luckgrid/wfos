//! Exact-key environment snapshot for authorized children (never log values).

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
}

impl EnvError {
    pub fn diagnostic(&self) -> DiagnosticRecord {
        match self {
            Self::SecretKeyName { key } => DiagnosticRecord {
                code: "execution_contract".into(),
                message: format!("refusing secret-named environment key `{key}`"),
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
}

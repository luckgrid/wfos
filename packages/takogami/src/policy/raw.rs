//! Strict typed raw policy/profile enforcement shapes.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::registry::{PolicyRecord, ProfileRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum PolicyContractKind {
    PolicyContractInvalid,
    PolicyVersionUnsupported,
    PolicyRuleInvalid,
    PolicyPathPatternInvalid,
    PolicyInputMismatch,
}

impl PolicyContractKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::PolicyContractInvalid => "policy_contract_invalid",
            Self::PolicyVersionUnsupported => "policy_version_unsupported",
            Self::PolicyRuleInvalid => "policy_rule_invalid",
            Self::PolicyPathPatternInvalid => "policy_path_pattern_invalid",
            Self::PolicyInputMismatch => "policy_input_mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPolicyError {
    pub kind: PolicyContractKind,
    pub message: String,
    pub policy_id: Option<String>,
    pub field: Option<String>,
}

impl RawPolicyError {
    pub fn new(
        kind: PolicyContractKind,
        message: impl Into<String>,
        policy_id: Option<String>,
        field: Option<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            policy_id,
            field,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentRules {
    #[serde(default)]
    pub env_flag: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub block: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct RuleTable {
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityGates {
    #[serde(default)]
    pub no_global_install: Option<bool>,
    #[serde(default)]
    pub no_secret_read: Option<bool>,
    #[serde(default)]
    pub no_shell_mutation: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct SecretRules {
    #[serde(default)]
    pub block_tools: Vec<String>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteWritePolicy {
    Blocked,
    LocalOnly,
    Elevated,
}

impl RemoteWritePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::LocalOnly => "local-only",
            Self::Elevated => "elevated",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct RemoteRules {
    #[serde(default)]
    pub writes: Option<RemoteWritePolicy>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Rationale {
    #[serde(default)]
    pub text: Option<String>,
}

/// Strict enforcement body for a projected policy record.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyEnforcementRecord {
    pub id: String,
    pub applies_to: String,
    pub version: String,
    #[serde(default)]
    pub agent: Option<AgentRules>,
    #[serde(default)]
    pub allow: Option<RuleTable>,
    #[serde(default)]
    pub gate: Option<RuleTable>,
    #[serde(default)]
    pub block: Option<RuleTable>,
    #[serde(default)]
    pub gates: Option<CapabilityGates>,
    #[serde(default)]
    pub secrets: Option<SecretRules>,
    #[serde(default)]
    pub remote: Option<RemoteRules>,
    #[serde(default)]
    pub rationale: Option<Rationale>,
    /// Generated-only projection field.
    #[serde(default)]
    pub source: Option<String>,
}

impl PolicyEnforcementRecord {
    pub fn from_registry(record: &PolicyRecord) -> Result<Self, RawPolicyError> {
        let mut map = BTreeMap::new();
        map.insert("id".into(), Value::String(record.id.clone()));
        match &record.applies_to {
            Some(a) if !a.is_empty() => {
                map.insert("applies_to".into(), Value::String(a.clone()));
            }
            _ => {
                return Err(RawPolicyError::new(
                    PolicyContractKind::PolicyContractInvalid,
                    "policy missing applies_to",
                    Some(record.id.clone()),
                    Some("applies_to".into()),
                ));
            }
        }
        for (k, v) in &record.rest {
            map.insert(k.clone(), v.clone());
        }
        let value = Value::Object(map.into_iter().collect());
        let parsed: Self = serde_json::from_value(value).map_err(|e| {
            RawPolicyError::new(
                PolicyContractKind::PolicyRuleInvalid,
                format!("policy parse failed: {e}"),
                Some(record.id.clone()),
                None,
            )
        })?;
        parsed.validate_version()?;
        parsed.validate_actions()?;
        Ok(parsed)
    }

    fn validate_version(&self) -> Result<(), RawPolicyError> {
        let ok = self.version == "0.1"
            || self
                .version
                .strip_prefix("0.1.")
                .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));
        if ok {
            Ok(())
        } else {
            Err(RawPolicyError::new(
                PolicyContractKind::PolicyVersionUnsupported,
                format!("unsupported policy version `{}`", self.version),
                Some(self.id.clone()),
                Some("version".into()),
            ))
        }
    }

    fn validate_actions(&self) -> Result<(), RawPolicyError> {
        for table in [&self.allow, &self.gate, &self.block].into_iter().flatten() {
            for action in &table.actions {
                if action != "delete untracked files" {
                    return Err(RawPolicyError::new(
                        PolicyContractKind::PolicyRuleInvalid,
                        format!("unsupported action `{action}`"),
                        Some(self.id.clone()),
                        Some("actions".into()),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Typed profile enforcement view extracted from a flattened ProfileRecord.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileEnforcement {
    pub id: String,
    pub allowed_paths: Vec<String>,
    pub blocked_paths: Vec<String>,
    pub allowed_commands: Vec<String>,
    pub gated_commands: Vec<String>,
    pub blocked_commands: Vec<String>,
    pub secret_access: bool,
    pub remote_write_policy: RemoteWritePolicy,
    pub isolation_mode: Option<String>,
    pub isolation_jj: Option<String>,
}

impl ProfileEnforcement {
    pub fn from_record(profile: &ProfileRecord) -> Result<Self, RawPolicyError> {
        let allowed_paths = string_array(&profile.rest, "allowed_paths")?;
        if allowed_paths.is_empty() {
            return Err(RawPolicyError::new(
                PolicyContractKind::PolicyContractInvalid,
                "profile allowed_paths must be non-empty for routed commands",
                Some(profile.id.clone()),
                Some("allowed_paths".into()),
            ));
        }
        let blocked_paths = string_array(&profile.rest, "blocked_paths").unwrap_or_default();
        let allowed_commands = string_array(&profile.rest, "allowed_commands").unwrap_or_default();
        let gated_commands = string_array(&profile.rest, "gated_commands").unwrap_or_default();
        let blocked_commands = string_array(&profile.rest, "blocked_commands").unwrap_or_default();
        let secret_access = bool_field(&profile.rest, "secret_access")?.unwrap_or(false);
        let remote_write_policy = match profile
            .rest
            .get("remote_write_policy")
            .and_then(|v| v.as_str())
        {
            None => RemoteWritePolicy::Blocked,
            Some("blocked") => RemoteWritePolicy::Blocked,
            Some("local-only") => RemoteWritePolicy::LocalOnly,
            Some("elevated") => RemoteWritePolicy::Elevated,
            Some(other) => {
                return Err(RawPolicyError::new(
                    PolicyContractKind::PolicyRuleInvalid,
                    format!("unsupported remote_write_policy `{other}`"),
                    Some(profile.id.clone()),
                    Some("remote_write_policy".into()),
                ));
            }
        };
        Ok(Self {
            id: profile.id.clone(),
            allowed_paths,
            blocked_paths,
            allowed_commands,
            gated_commands,
            blocked_commands,
            secret_access,
            remote_write_policy,
            isolation_mode: profile.isolation_mode.clone(),
            isolation_jj: profile.isolation_jj.clone(),
        })
    }
}

fn string_array(map: &BTreeMap<String, Value>, key: &str) -> Result<Vec<String>, RawPolicyError> {
    match map.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(RawPolicyError::new(
                            PolicyContractKind::PolicyRuleInvalid,
                            format!("{key} items must be strings"),
                            None,
                            Some(key.into()),
                        ));
                    }
                }
            }
            Ok(out)
        }
        Some(_) => Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            format!("{key} must be an array of strings"),
            None,
            Some(key.into()),
        )),
    }
}

fn bool_field(map: &BTreeMap<String, Value>, key: &str) -> Result<Option<bool>, RawPolicyError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            format!("{key} must be a boolean"),
            None,
            Some(key.into()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_git_shape() {
        let record = PolicyRecord {
            id: "agent-git".into(),
            applies_to: Some("agent".into()),
            rest: serde_json::from_value(serde_json::json!({
                "version": "0.1.0",
                "allow": {"commands": ["git status"]},
                "gate": {"commands": ["git commit"]},
                "block": {"commands": ["git push"], "actions": ["delete untracked files"]},
                "remote": {"writes": "elevated"}
            }))
            .unwrap(),
        };
        let parsed = PolicyEnforcementRecord::from_registry(&record).unwrap();
        assert_eq!(parsed.allow.unwrap().commands, vec!["git status"]);
        assert_eq!(
            parsed.remote.unwrap().writes,
            Some(RemoteWritePolicy::Elevated)
        );
    }

    #[test]
    fn rejects_unknown_rule_field() {
        let record = PolicyRecord {
            id: "bad".into(),
            applies_to: Some("agent".into()),
            rest: serde_json::from_value(serde_json::json!({
                "version": "0.1.0",
                "allow": {"command": ["x"]}
            }))
            .unwrap(),
        };
        assert!(PolicyEnforcementRecord::from_registry(&record).is_err());
    }
}

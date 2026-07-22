//! Registry JSON shapes projected by Ontarch (subset needed for S3/S4).

use std::collections::BTreeMap;

use crate::contracts::ExecutionClass;
use crate::contracts::RegistryGeneration;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Hit,
    Miss,
    Stale,
}

impl Freshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Stale => "stale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryFileKind {
    Units,
    Scan,
    Tools,
    Profiles,
    Policies,
    Graph,
    Manifest,
}

impl RegistryFileKind {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Units => "units.json",
            Self::Scan => "scan.json",
            Self::Tools => "tools.json",
            Self::Profiles => "profiles.json",
            Self::Policies => "policies.json",
            Self::Graph => "graph.json",
            Self::Manifest => "manifest.json",
        }
    }
}

/// Lifecycle verb → command, matching Ontarch structured/legacy entrypoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum EntrypointDefinition {
    Legacy(String),
    Structured(StructuredEntrypoint),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuredEntrypoint {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_manifests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_policies: Vec<String>,
    #[serde(default)]
    pub execution_class: ExecutionClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnitsDocument {
    pub generated_at: String,
    #[serde(default)]
    pub registry_generation: Option<RegistryGeneration>,
    #[serde(default)]
    pub summary: Value,
    #[serde(default)]
    pub units: Vec<UnitRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnitRecord {
    pub id: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub layer: Option<String>,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub native_manifests: Vec<String>,
    #[serde(default)]
    pub entrypoints: BTreeMap<String, EntrypointDefinition>,
    #[serde(default)]
    pub cli: Option<Value>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub policy: Option<Value>,
    #[serde(default)]
    pub source: Option<String>,
    /// True when synthesized from scan evidence without a descriptor.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provisional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_complete: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScanDocument {
    pub generated_at: String,
    #[serde(default)]
    pub registry_generation: Option<RegistryGeneration>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub summary: Value,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceScanEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceScanEntry {
    pub path: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub git_root: Option<String>,
    #[serde(default)]
    pub native_manifests: Vec<String>,
    /// Evidence only — never auto-executed (E09.S3).
    #[serde(default)]
    pub lint_check_commands: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolsDocument {
    pub generated_at: String,
    #[serde(default)]
    pub summary: Value,
    #[serde(default)]
    pub tools: Vec<ToolRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolRecord {
    pub id: String,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub installed: Option<bool>,
    #[serde(default)]
    pub default: Option<bool>,
    #[serde(default)]
    pub agent_safe: Option<bool>,
    #[serde(default)]
    pub detect: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvisionalUnit {
    pub unit: UnitRecord,
    pub evidence: Vec<String>,
}

/// Flattened profile projection (matches `profiles.json` top-level fields).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfilesDocument {
    pub generated_at: String,
    #[serde(default)]
    pub profiles: Vec<ProfileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileRecord {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub rails: Option<String>,
    #[serde(default)]
    pub rails_bin: Option<String>,
    #[serde(default)]
    pub isolation_mode: Option<String>,
    #[serde(default)]
    pub isolation_jj: Option<String>,
    #[serde(default)]
    pub session_state_home: Option<String>,
    /// Remainder of the flattened projection (paths, commands, skills, …).
    #[serde(flatten)]
    pub rest: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PoliciesDocument {
    pub generated_at: String,
    #[serde(default)]
    pub policies: Vec<PolicyRecord>,
}

/// S4 needs id + applies_to; bodies stay opaque for S5.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyRecord {
    pub id: String,
    #[serde(default)]
    pub applies_to: Option<String>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, Value>,
}

/// Authored descriptor TOML (source-authoritative routing).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthoredUnitDescriptor {
    pub id: String,
    #[serde(default)]
    pub paths: Option<AuthoredPaths>,
    #[serde(default)]
    pub native: Option<AuthoredNative>,
    #[serde(default)]
    pub entrypoints: BTreeMap<String, EntrypointDefinition>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthoredPaths {
    pub root: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthoredNative {
    #[serde(default)]
    pub manifests: Vec<String>,
}

/// Normalized unit definition shared by hit projections and authored TOML.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitDefinition {
    pub id: String,
    pub path: Option<String>,
    pub root: Option<String>,
    pub native_manifests: Vec<String>,
    pub entrypoints: BTreeMap<String, EntrypointDefinition>,
    pub descriptor_path: String,
    pub provisional: bool,
    pub routing_complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_entrypoint_rejects_unknown_projection_fields() {
        let value = serde_json::json!({
            "program": "moon",
            "arg": ["run", "demo:build"]
        });
        assert!(serde_json::from_value::<StructuredEntrypoint>(value).is_err());
    }

    #[test]
    fn structured_entrypoint_rejects_unknown_authored_fields() {
        let text = r#"
id = "demo"
[entrypoints.build]
program = "moon"
arg = ["run", "demo:build"]
"#;
        assert!(toml::from_str::<AuthoredUnitDescriptor>(text).is_err());
    }
}

//! Registry JSON shapes projected by Ontarch (subset needed for S3 queries).

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
    pub entrypoints: Value,
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

//! Closed graph document types (deny_unknown_fields; schema-parity enums).

use crate::contracts::RegistryGeneration;
use serde::{Deserialize, Serialize};

/// Maximum accepted graph.json size (8 MiB).
pub const GRAPH_FILE_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum node count.
pub const GRAPH_NODE_LIMIT: usize = 20_000;
/// Maximum edge count.
pub const GRAPH_EDGE_LIMIT: usize = 100_000;
/// Maximum UTF-8 byte length for node/edge endpoint IDs.
pub const GRAPH_ID_LIMIT_BYTES: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GraphDocument {
    pub generated_at: String,
    pub registry_generation: RegistryGeneration,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub id: String,
    pub kind: GraphNodeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub from: String,
    pub rel: GraphRelation,
    pub to: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum GraphNodeKind {
    Workspace,
    #[serde(rename = "native-toolchain")]
    NativeToolchain,
    #[serde(rename = "metadata-plane")]
    MetadataPlane,
    #[serde(rename = "runtime-controller")]
    RuntimeController,
    #[serde(rename = "package-translator")]
    PackageTranslator,
    #[serde(rename = "portable-component-runtime")]
    PortableComponentRuntime,
    Package,
    App,
    Site,
    Pattern,
    Tool,
    Runtime,
    Agent,
    Policy,
    Capability,
    Actor,
    Profile,
    Skill,
}

impl GraphNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::NativeToolchain => "native-toolchain",
            Self::MetadataPlane => "metadata-plane",
            Self::RuntimeController => "runtime-controller",
            Self::PackageTranslator => "package-translator",
            Self::PortableComponentRuntime => "portable-component-runtime",
            Self::Package => "package",
            Self::App => "app",
            Self::Site => "site",
            Self::Pattern => "pattern",
            Self::Tool => "tool",
            Self::Runtime => "runtime",
            Self::Agent => "agent",
            Self::Policy => "policy",
            Self::Capability => "capability",
            Self::Actor => "actor",
            Self::Profile => "profile",
            Self::Skill => "skill",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum GraphRelation {
    Provides,
    Requires,
    Uses,
    #[serde(rename = "governed-by")]
    GovernedBy,
    #[serde(rename = "blocked-by")]
    BlockedBy,
    #[serde(rename = "packaged-by")]
    PackagedBy,
    #[serde(rename = "runs-on")]
    RunsOn,
    Selects,
    #[serde(rename = "can-invoke")]
    CanInvoke,
}

impl GraphRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Provides => "provides",
            Self::Requires => "requires",
            Self::Uses => "uses",
            Self::GovernedBy => "governed-by",
            Self::BlockedBy => "blocked-by",
            Self::PackagedBy => "packaged-by",
            Self::RunsOn => "runs-on",
            Self::Selects => "selects",
            Self::CanInvoke => "can-invoke",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_root_field() {
        let raw = r#"{"generated_at":"2026-07-25T00:00:00Z","registry_generation":{"generated_at":"2026-07-25T00:00:00Z","source_fingerprints":[]},"nodes":[],"edges":[],"extra":1}"#;
        assert!(serde_json::from_str::<GraphDocument>(raw).is_err());
    }

    #[test]
    fn rejects_unknown_node_kind() {
        let raw = r#"{"generated_at":"2026-07-25T00:00:00Z","registry_generation":{"generated_at":"2026-07-25T00:00:00Z","source_fingerprints":[]},"nodes":[{"id":"x","kind":"tendril"}],"edges":[]}"#;
        assert!(serde_json::from_str::<GraphDocument>(raw).is_err());
    }

    #[test]
    fn accepts_schema_kinds_and_rels() {
        let raw = r#"{
          "generated_at":"2026-07-25T00:00:00Z",
          "registry_generation":{"generated_at":"2026-07-25T00:00:00Z","source_fingerprints":[]},
          "nodes":[{"id":"a","kind":"package"},{"id":"b","kind":"policy"}],
          "edges":[{"from":"a","rel":"governed-by","to":"b"}]
        }"#;
        let doc: GraphDocument = serde_json::from_str(raw).unwrap();
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.edges[0].rel, GraphRelation::GovernedBy);
    }
}

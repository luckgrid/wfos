//! Graph semantic validation and layered freshness.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::contracts::{fingerprint_bytes, parse_rfc3339_utc_seconds};
use crate::error::ControllerError;
use crate::registry::{Freshness, RegistryAccess, RegistryFileKind};

use super::io::{SecureFileError, read_bounded_nofollow, sha256_regular_nofollow};
use super::types::{
    GRAPH_EDGE_LIMIT, GRAPH_FILE_LIMIT_BYTES, GRAPH_FRESHNESS_METADATA_LIMIT_BYTES,
    GRAPH_ID_LIMIT_BYTES, GRAPH_NODE_LIMIT, GraphDocument, GraphRegistryGeneration,
};

/// Canonical Layer-1 fingerprint paths (sorted).
pub const GRAPH_UPSTREAM_PATHS: [&str; 4] = [
    "registry/policies.json",
    "registry/profiles.json",
    "registry/skills.json",
    "registry/units.json",
];

#[derive(Debug, Clone)]
pub struct GraphLoadOutcome {
    pub document: GraphDocument,
    pub freshness: Freshness,
}

/// Closed Layer-2 view of units.json freshness metadata only.
#[derive(Debug, Deserialize)]
struct UnitsFreshnessView {
    #[serde(default)]
    registry_generation: Option<GraphRegistryGeneration>,
}

/// Load, validate, and freshness-check `registry/graph.json` (no-follow, bounded).
pub fn load_graph(access: &RegistryAccess) -> Result<GraphLoadOutcome, ControllerError> {
    let path = access.file_path(RegistryFileKind::Graph);
    const DISPLAY: &str = "registry/graph.json";
    let buf = match read_bounded_nofollow(&path, DISPLAY, GRAPH_FILE_LIMIT_BYTES) {
        Ok(b) => b,
        Err(e) => return Err(map_graph_file_err(e)),
    };

    let mut doc: GraphDocument = serde_json::from_slice(&buf).map_err(|e| {
        ControllerError::graph_contract_invalid(format!("malformed graph.json: {e}"))
    })?;
    validate_graph_document(&mut doc)?;
    match evaluate_graph_freshness(access, &doc)? {
        Freshness::Hit => Ok(GraphLoadOutcome {
            document: doc,
            freshness: Freshness::Hit,
        }),
        Freshness::Stale => Err(ControllerError::graph_stale(
            "graph upstream or authored unit fingerprints are stale",
        )),
        Freshness::Miss => Err(ControllerError::graph_missing()),
    }
}

fn map_graph_file_err(e: SecureFileError) -> ControllerError {
    match e {
        SecureFileError::Missing => ControllerError::graph_missing(),
        SecureFileError::Limit { limit } => ControllerError::graph_limit_exceeded(format!(
            "registry/graph.json exceeds {limit} byte limit"
        )),
        SecureFileError::Symlink => {
            ControllerError::graph_contract_invalid("registry/graph.json must not be a symlink")
        }
        SecureFileError::NonRegular => ControllerError::graph_contract_invalid(
            "registry/graph.json must be a regular non-symlink file",
        ),
        SecureFileError::Io { .. } => ControllerError::graph_contract_invalid(format!(
            "registry/graph.json: {}",
            e.public_message()
        )),
    }
}

/// Map freshness hash errors: Missing ? Ok (caller treats as stale); else contract Err.
fn map_freshness_hash_err(
    e: SecureFileError,
    non_regular_msg: &str,
) -> Result<(), ControllerError> {
    match e {
        SecureFileError::Missing => Ok(()),
        SecureFileError::Symlink | SecureFileError::NonRegular => {
            Err(ControllerError::graph_contract_invalid(non_regular_msg))
        }
        SecureFileError::Limit { .. } => Err(ControllerError::graph_contract_invalid(format!(
            "{non_regular_msg}: {}",
            e.public_message()
        ))),
        SecureFileError::Io { .. } => {
            Err(ControllerError::graph_contract_invalid(e.public_message()))
        }
    }
}

fn map_units_metadata_err(e: SecureFileError) -> Result<Freshness, ControllerError> {
    match e {
        SecureFileError::Missing => Ok(Freshness::Stale),
        SecureFileError::Limit { limit } => Err(ControllerError::graph_limit_exceeded(format!(
            "registry/units.json exceeds {limit} byte limit"
        ))),
        SecureFileError::Symlink => Err(ControllerError::graph_contract_invalid(
            "registry/units.json must not be a symlink",
        )),
        SecureFileError::NonRegular => Err(ControllerError::graph_contract_invalid(
            "registry/units.json must be a regular non-symlink file",
        )),
        SecureFileError::Io { .. } => Err(ControllerError::graph_contract_invalid(format!(
            "registry/units.json: {}",
            e.public_message()
        ))),
    }
}

/// Validate semantics/limits and sort nodes/edges in place.
pub fn validate_graph_document(doc: &mut GraphDocument) -> Result<(), ControllerError> {
    require_graph_utc_seconds(&doc.generated_at, "generated_at")?;
    require_graph_utc_seconds(
        &doc.registry_generation.generated_at,
        "registry_generation.generated_at",
    )?;
    validate_fingerprint_shape(&doc.registry_generation)?;

    if doc.nodes.len() > GRAPH_NODE_LIMIT {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "node count {} exceeds limit {GRAPH_NODE_LIMIT}",
            doc.nodes.len()
        )));
    }
    if doc.edges.len() > GRAPH_EDGE_LIMIT {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "edge count {} exceeds limit {GRAPH_EDGE_LIMIT}",
            doc.edges.len()
        )));
    }

    let mut seen_ids = HashSet::new();
    for node in &doc.nodes {
        check_id(&node.id, "node id")?;
        if !seen_ids.insert(node.id.as_str()) {
            return Err(ControllerError::graph_contract_invalid(format!(
                "duplicate node id '{}'",
                truncate_id(&node.id)
            )));
        }
    }

    let mut seen_edges = HashSet::new();
    for edge in &doc.edges {
        check_id(&edge.from, "edge from")?;
        check_id(&edge.to, "edge to")?;
        check_id(edge.rel.as_str(), "edge rel")?;
        if !seen_ids.contains(edge.from.as_str()) {
            return Err(ControllerError::graph_endpoint_invalid(format!(
                "missing edge source '{}'",
                truncate_id(&edge.from)
            )));
        }
        if !seen_ids.contains(edge.to.as_str()) {
            return Err(ControllerError::graph_endpoint_invalid(format!(
                "missing edge target '{}'",
                truncate_id(&edge.to)
            )));
        }
        let key = (edge.from.as_str(), edge.rel.as_str(), edge.to.as_str());
        if !seen_edges.insert(key) {
            return Err(ControllerError::graph_contract_invalid(
                "duplicate edge tuple",
            ));
        }
    }

    canonicalize_graph(doc);
    Ok(())
}

pub fn canonicalize_graph(doc: &mut GraphDocument) {
    doc.nodes
        .sort_by(|a, b| (a.kind, a.id.as_str()).cmp(&(b.kind, b.id.as_str())));
    doc.edges.sort_by(|a, b| {
        (a.from.as_str(), a.rel.as_str(), a.to.as_str()).cmp(&(
            b.from.as_str(),
            b.rel.as_str(),
            b.to.as_str(),
        ))
    });
}

fn validate_fingerprint_shape(generation: &GraphRegistryGeneration) -> Result<(), ControllerError> {
    let fps = &generation.source_fingerprints;
    if fps.len() != 4 {
        return Err(ControllerError::graph_contract_invalid(
            "graph fingerprint count must be exactly 4",
        ));
    }
    let mut paths = Vec::with_capacity(4);
    let mut uniq = BTreeSet::new();
    for fp in fps {
        if fp.algorithm != "sha256" {
            return Err(ControllerError::graph_contract_invalid(
                "unsupported fingerprint algorithm",
            ));
        }
        if !is_sha256_hex(&fp.digest) {
            return Err(ControllerError::graph_contract_invalid(
                "malformed fingerprint digest",
            ));
        }
        if fp.path.starts_with('/') || fp.path.contains('\\') || fp.path.contains("..") {
            return Err(ControllerError::graph_contract_invalid(
                "fingerprint path must be relative and non-traversing",
            ));
        }
        if !uniq.insert(fp.path.as_str()) {
            return Err(ControllerError::graph_contract_invalid(
                "duplicate fingerprint path",
            ));
        }
        paths.push(fp.path.as_str());
    }
    let expected: Vec<&str> = GRAPH_UPSTREAM_PATHS.to_vec();
    if paths != expected {
        let mut sorted = paths.clone();
        sorted.sort();
        if sorted == expected {
            return Err(ControllerError::graph_contract_invalid(
                "fingerprint paths unsorted",
            ));
        }
        return Err(ControllerError::graph_contract_invalid(
            "fingerprint path set must be policies/profiles/skills/units",
        ));
    }
    Ok(())
}

/// Resolve Phase 1 package-relative `registry/*.json` against `registry_root`.
pub fn resolve_graph_upstream(
    registry_root: &Path,
    recorded: &str,
) -> Result<PathBuf, ControllerError> {
    let name = recorded.strip_prefix("registry/").ok_or_else(|| {
        ControllerError::graph_contract_invalid(
            "graph upstream fingerprint path must start with registry/",
        )
    })?;
    if !matches!(
        name,
        "policies.json" | "profiles.json" | "skills.json" | "units.json"
    ) {
        return Err(ControllerError::graph_contract_invalid(
            "graph upstream fingerprint path set must be policies/profiles/skills/units",
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(ControllerError::graph_contract_invalid(
            "fingerprint path must be relative and non-traversing",
        ));
    }
    Ok(registry_root.join(name))
}

/// Layer 1 (graph->upstream via registry_root) + Layer 2 (units authored via workspace_root).
pub fn evaluate_graph_freshness(
    access: &RegistryAccess,
    doc: &GraphDocument,
) -> Result<Freshness, ControllerError> {
    for fp in &doc.registry_generation.source_fingerprints {
        let abs = resolve_graph_upstream(&access.paths.registry_root, &fp.path)?;
        let current = match sha256_regular_nofollow(&abs, &fp.path) {
            Ok(c) => c,
            Err(e) => {
                map_freshness_hash_err(
                    e,
                    &format!(
                        "upstream fingerprint target must be a regular non-symlink file ({})",
                        fp.path
                    ),
                )?;
                return Ok(Freshness::Stale);
            }
        };
        if current.digest != fp.digest || current.algorithm != fp.algorithm {
            return Ok(Freshness::Stale);
        }
    }

    let units_path = access.file_path(RegistryFileKind::Units);
    let units_bytes = match read_bounded_nofollow(
        &units_path,
        "registry/units.json",
        GRAPH_FRESHNESS_METADATA_LIMIT_BYTES,
    ) {
        Ok(b) => b,
        Err(e) => return map_units_metadata_err(e),
    };
    let units_view: UnitsFreshnessView = serde_json::from_slice(&units_bytes).map_err(|e| {
        ControllerError::graph_contract_invalid(format!("malformed units.json: {e}"))
    })?;
    let Some(authored) = units_view.registry_generation else {
        return Ok(Freshness::Stale);
    };
    require_graph_utc_seconds(
        &authored.generated_at,
        "units.registry_generation.generated_at",
    )?;
    if authored.source_fingerprints.is_empty() {
        return Ok(Freshness::Stale);
    }
    for fp in &authored.source_fingerprints {
        if fp.algorithm != "sha256" || !is_sha256_hex(&fp.digest) {
            return Err(ControllerError::graph_contract_invalid(
                "authored fingerprint metadata invalid",
            ));
        }
        let abs = resolve_confined(&access.paths.workspace_root, &fp.path)?;
        let current = match sha256_regular_nofollow(&abs, &fp.path) {
            Ok(c) => c,
            Err(e) => {
                map_freshness_hash_err(
                    e,
                    &format!(
                        "authored source must be a regular non-symlink file ({})",
                        fp.path
                    ),
                )?;
                return Ok(Freshness::Stale);
            }
        };
        if current.digest != fp.digest {
            return Ok(Freshness::Stale);
        }
    }
    Ok(Freshness::Hit)
}

fn resolve_confined(workspace_root: &Path, recorded: &str) -> Result<PathBuf, ControllerError> {
    if recorded.is_empty()
        || recorded.starts_with('/')
        || recorded.contains('\\')
        || recorded.contains("..")
    {
        return Err(ControllerError::graph_contract_invalid(
            "fingerprint path must be relative and non-traversing",
        ));
    }
    let joined = workspace_root.join(recorded);
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    if let Some(parent) = joined.parent()
        && let Ok(canon_parent) = parent.canonicalize()
        && !canon_parent.starts_with(&root)
    {
        return Err(ControllerError::graph_contract_invalid(
            "fingerprint path escapes workspace root",
        ));
    }
    Ok(joined)
}

fn require_graph_utc_seconds(ts: &str, field: &str) -> Result<(), ControllerError> {
    // Exact graph lexical form YYYY-MM-DDTHH:MM:SSZ, then calendar-valid parse.
    let lexical = ts.len() == 20
        && ts.as_bytes().get(4) == Some(&b'-')
        && ts.as_bytes().get(7) == Some(&b'-')
        && ts.as_bytes().get(10) == Some(&b'T')
        && ts.as_bytes().get(13) == Some(&b':')
        && ts.as_bytes().get(16) == Some(&b':')
        && ts.ends_with('Z')
        && ts[..4].bytes().all(|b| b.is_ascii_digit())
        && ts[5..7].bytes().all(|b| b.is_ascii_digit())
        && ts[8..10].bytes().all(|b| b.is_ascii_digit())
        && ts[11..13].bytes().all(|b| b.is_ascii_digit())
        && ts[14..16].bytes().all(|b| b.is_ascii_digit())
        && ts[17..19].bytes().all(|b| b.is_ascii_digit());
    if !lexical {
        return Err(ControllerError::graph_contract_invalid(format!(
            "invalid {field} timestamp"
        )));
    }
    parse_rfc3339_utc_seconds(ts).map_err(|_| {
        ControllerError::graph_contract_invalid(format!("invalid {field} timestamp"))
    })?;
    Ok(())
}

fn check_id(id: &str, label: &str) -> Result<(), ControllerError> {
    if id.is_empty() {
        return Err(ControllerError::graph_contract_invalid(format!(
            "{label} must be non-empty"
        )));
    }
    if id.len() > GRAPH_ID_LIMIT_BYTES {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "{label} exceeds {GRAPH_ID_LIMIT_BYTES} byte limit"
        )));
    }
    if id.chars().any(|c| {
        let u = c as u32;
        u < 32 || u == 127
    }) {
        return Err(ControllerError::graph_contract_invalid(format!(
            "{label} contains control characters"
        )));
    }
    Ok(())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn truncate_id(id: &str) -> String {
    const MAX_CHARS: usize = 60;
    let mut chars = id.chars();
    let prefix: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{prefix}\u{2026}")
    } else {
        prefix
    }
}

/// Helper for tests: digest of bytes as lowercase hex (no path).
#[allow(dead_code)]
pub fn digest_hex(data: &[u8]) -> String {
    fingerprint_bytes(data).digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{
        GraphEdge, GraphNode, GraphNodeKind, GraphRelation, GraphSourceFingerprint,
    };

    fn empty_gen() -> GraphRegistryGeneration {
        GraphRegistryGeneration {
            generated_at: "2026-07-25T00:00:00Z".into(),
            source_fingerprints: GRAPH_UPSTREAM_PATHS
                .iter()
                .map(|p| GraphSourceFingerprint {
                    path: (*p).into(),
                    algorithm: "sha256".into(),
                    digest: "ab".repeat(32),
                })
                .collect(),
        }
    }

    #[test]
    fn sorts_nodes_and_edges() {
        let mut doc = GraphDocument {
            generated_at: "2026-07-25T00:00:00Z".into(),
            registry_generation: empty_gen(),
            nodes: vec![
                GraphNode {
                    id: "z".into(),
                    kind: GraphNodeKind::Package,
                },
                GraphNode {
                    id: "a".into(),
                    kind: GraphNodeKind::Package,
                },
            ],
            edges: vec![
                GraphEdge {
                    from: "z".into(),
                    rel: GraphRelation::Uses,
                    to: "a".into(),
                },
                GraphEdge {
                    from: "a".into(),
                    rel: GraphRelation::Uses,
                    to: "z".into(),
                },
            ],
        };
        validate_graph_document(&mut doc).unwrap();
        assert_eq!(doc.nodes[0].id, "a");
        assert_eq!(doc.edges[0].from, "a");
    }

    #[test]
    fn rejects_duplicate_node() {
        let mut doc = GraphDocument {
            generated_at: "2026-07-25T00:00:00Z".into(),
            registry_generation: empty_gen(),
            nodes: vec![
                GraphNode {
                    id: "a".into(),
                    kind: GraphNodeKind::Package,
                },
                GraphNode {
                    id: "a".into(),
                    kind: GraphNodeKind::Policy,
                },
            ],
            edges: vec![],
        };
        let err = validate_graph_document(&mut doc).unwrap_err();
        assert_eq!(err.diagnostic_code(), "graph_contract_invalid");
    }

    #[test]
    fn resolve_graph_upstream_strips_registry_prefix() {
        let root = PathBuf::from("/tmp/reg");
        let p = resolve_graph_upstream(&root, "registry/policies.json").unwrap();
        assert_eq!(p, root.join("policies.json"));
    }

    #[test]
    fn rejects_calendar_invalid_timestamp() {
        let mut doc = GraphDocument {
            generated_at: "2026-13-40T99:99:99Z".into(),
            registry_generation: empty_gen(),
            nodes: vec![],
            edges: vec![],
        };
        let err = validate_graph_document(&mut doc).unwrap_err();
        assert_eq!(err.diagnostic_code(), "graph_contract_invalid");
    }

    #[test]
    fn accepts_valid_leap_day() {
        let mut generation = empty_gen();
        generation.generated_at = "2024-02-29T12:00:00Z".into();
        let mut doc = GraphDocument {
            generated_at: "2024-02-29T12:00:00Z".into(),
            registry_generation: generation,
            nodes: vec![],
            edges: vec![],
        };
        validate_graph_document(&mut doc).unwrap();
    }

    #[test]
    fn truncate_id_is_utf8_safe_for_emoji() {
        // 70 emoji chars (>60 char truncate threshold; also >64 bytes).
        let id = "\u{1F44D}".repeat(70);
        assert!(id.len() > 64);
        let t = truncate_id(&id);
        assert!(t.ends_with('\u{2026}'));
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }

    fn io_open_fail(
        _physical: &Path,
        display: &str,
    ) -> Result<std::fs::File, super::super::io::SecureFileError> {
        Err(super::super::io::SecureFileError::io(
            super::super::io::SecureFileOperation::Open,
            display,
            std::io::Error::other("injected"),
        ))
    }

    #[test]
    fn present_regular_upstream_io_failure_is_contract_not_stale() {
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("registry");
        std::fs::create_dir_all(&registry).unwrap();
        for name in [
            "policies.json",
            "profiles.json",
            "skills.json",
            "units.json",
        ] {
            std::fs::write(registry.join(name), b"{}").unwrap();
        }
        let access = crate::registry::RegistryAccess::new(crate::registry::RegistryPaths {
            registry_root: registry,
            workspace_root: dir.path().to_path_buf(),
        });
        let fps: Vec<GraphSourceFingerprint> = GRAPH_UPSTREAM_PATHS
            .iter()
            .map(|p| GraphSourceFingerprint {
                path: (*p).into(),
                algorithm: "sha256".into(),
                digest: "ab".repeat(32),
            })
            .collect();
        let doc = GraphDocument {
            generated_at: "2026-07-25T00:00:00Z".into(),
            registry_generation: GraphRegistryGeneration {
                generated_at: "2026-07-25T00:00:00Z".into(),
                source_fingerprints: fps,
            },
            nodes: vec![],
            edges: vec![],
        };
        super::super::io::set_open_override(Some(io_open_fail));
        let err = evaluate_graph_freshness(&access, &doc).unwrap_err();
        super::super::io::set_open_override(None);
        assert_eq!(err.diagnostic_code(), "graph_contract_invalid");
        let root = dir.path().to_str().unwrap();
        assert!(
            !err.to_string().contains(root),
            "must not leak physical root: {}",
            err
        );
    }
}

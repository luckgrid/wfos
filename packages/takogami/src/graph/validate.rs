//! Graph semantic validation and layered freshness.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::contracts::{RegistryGeneration, fingerprint_bytes, fingerprint_file};
use crate::error::ControllerError;
use crate::registry::{Freshness, RegistryAccess, RegistryFileKind, UnitsDocument};

use super::types::{
    GRAPH_EDGE_LIMIT, GRAPH_FILE_LIMIT_BYTES, GRAPH_ID_LIMIT_BYTES, GRAPH_NODE_LIMIT, GraphDocument,
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

/// Load, validate, and freshness-check `registry/graph.json` (no-follow, bounded).
pub fn load_graph(access: &RegistryAccess) -> Result<GraphLoadOutcome, ControllerError> {
    let path = access.file_path(RegistryFileKind::Graph);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ControllerError::graph_missing());
        }
        Err(e) => {
            return Err(ControllerError::graph_contract_invalid(format!(
                "cannot stat graph.json: {e}"
            )));
        }
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        return Err(ControllerError::graph_contract_invalid(
            "graph.json must not be a symlink",
        ));
    }
    if !ft.is_file() {
        return Err(ControllerError::graph_contract_invalid(
            "graph.json must be a regular file",
        ));
    }
    if meta.len() > GRAPH_FILE_LIMIT_BYTES {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "graph.json exceeds {} byte limit",
            GRAPH_FILE_LIMIT_BYTES
        )));
    }

    let mut file = open_nofollow_read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ControllerError::graph_missing()
        } else {
            ControllerError::graph_contract_invalid(format!("cannot open graph.json: {e}"))
        }
    })?;
    // Recheck after open when metadata is available.
    if let Ok(opened) = file.metadata()
        && opened.len() > GRAPH_FILE_LIMIT_BYTES
    {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "graph.json exceeds {} byte limit",
            GRAPH_FILE_LIMIT_BYTES
        )));
    }

    let mut buf = Vec::new();
    let limit = (GRAPH_FILE_LIMIT_BYTES as usize).saturating_add(1);
    let mut take = (&mut file).take(limit as u64);
    take.read_to_end(&mut buf).map_err(|e| {
        ControllerError::graph_contract_invalid(format!("cannot read graph.json: {e}"))
    })?;
    if buf.len() > GRAPH_FILE_LIMIT_BYTES as usize {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "graph.json exceeds {} byte limit",
            GRAPH_FILE_LIMIT_BYTES
        )));
    }

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

fn open_nofollow_read(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Validate semantics/limits and sort nodes/edges in place.
pub fn validate_graph_document(doc: &mut GraphDocument) -> Result<(), ControllerError> {
    require_utc_seconds(&doc.generated_at, "generated_at")?;
    require_utc_seconds(
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

fn validate_fingerprint_shape(generation: &RegistryGeneration) -> Result<(), ControllerError> {
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
        // Distinguish unsorted vs wrong set.
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

/// Layer 1 (graph→upstream) + Layer 2 (units authored) freshness.
pub fn evaluate_graph_freshness(
    access: &RegistryAccess,
    doc: &GraphDocument,
) -> Result<Freshness, ControllerError> {
    // Layer 1: exact upstream bytes under workspace_root.
    for fp in &doc.registry_generation.source_fingerprints {
        let abs = resolve_confined(&access.paths.workspace_root, &fp.path)?;
        let meta = match fs::symlink_metadata(&abs) {
            Ok(m) => m,
            Err(_) => return Ok(Freshness::Stale),
        };
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(ControllerError::graph_contract_invalid(format!(
                "upstream fingerprint target must be a regular non-symlink file ({})",
                fp.path
            )));
        }
        let current = fingerprint_file(&abs, &fp.path).map_err(|e| {
            ControllerError::graph_contract_invalid(format!("cannot fingerprint {}: {e}", fp.path))
        })?;
        if current.digest != fp.digest || current.algorithm != fp.algorithm {
            return Ok(Freshness::Stale);
        }
    }

    // Layer 2: units.json.registry_generation authored sources.
    let units_path = access.file_path(RegistryFileKind::Units);
    let units_meta = match fs::symlink_metadata(&units_path) {
        Ok(m) => m,
        Err(_) => return Ok(Freshness::Stale),
    };
    if units_meta.file_type().is_symlink() || !units_meta.file_type().is_file() {
        return Err(ControllerError::graph_contract_invalid(
            "units.json must be a regular non-symlink file",
        ));
    }
    let units_text = fs::read_to_string(&units_path).map_err(|e| {
        ControllerError::graph_contract_invalid(format!("cannot read units.json: {e}"))
    })?;
    let units: UnitsDocument = serde_json::from_str(&units_text).map_err(|e| {
        ControllerError::graph_contract_invalid(format!("malformed units.json: {e}"))
    })?;
    let Some(authored) = units.registry_generation.as_ref() else {
        // Phase 2 fixture contract: missing authored generation ⇒ stale.
        return Ok(Freshness::Stale);
    };
    require_utc_seconds(
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
        let meta = match fs::symlink_metadata(&abs) {
            Ok(m) => m,
            Err(_) => return Ok(Freshness::Stale),
        };
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(ControllerError::graph_contract_invalid(
                "authored source must be a regular non-symlink file",
            ));
        }
        let current = fingerprint_file(&abs, &fp.path).map_err(|e| {
            ControllerError::graph_contract_invalid(format!(
                "cannot fingerprint authored {}: {e}",
                fp.path
            ))
        })?;
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
    // Best-effort confinement without following the target (parent canonicalize).
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

fn require_utc_seconds(ts: &str, field: &str) -> Result<(), ControllerError> {
    // YYYY-MM-DDTHH:MM:SSZ
    let ok = ts.len() == 20
        && ts.as_bytes().get(10) == Some(&b'T')
        && ts.ends_with('Z')
        && ts
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b':' | b'T' | b'Z'));
    if !ok {
        return Err(ControllerError::graph_contract_invalid(format!(
            "invalid {field} timestamp"
        )));
    }
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
    if id.len() <= 64 {
        id.to_string()
    } else {
        format!("{}…", &id[..61])
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
    use crate::contracts::{RegistryGeneration, SourceFingerprint};
    use crate::graph::types::{GraphEdge, GraphNode, GraphNodeKind, GraphRelation};

    fn empty_gen() -> RegistryGeneration {
        RegistryGeneration {
            generated_at: "2026-07-25T00:00:00Z".into(),
            source_fingerprints: GRAPH_UPSTREAM_PATHS
                .iter()
                .map(|p| SourceFingerprint {
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
}

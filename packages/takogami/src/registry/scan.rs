//! Scan discovery adapter over Ontarch scan.json (+ units for descriptor-backed matching).

use std::collections::HashMap;
use std::path::Path;

use crate::error::ControllerError;

use super::types::{Freshness, ScanDocument, UnitRecord, UnitsDocument};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanDiscovery {
    pub freshness: Freshness,
    pub workspaces: Vec<serde_json::Value>,
    pub units: Vec<UnitRecord>,
    pub provisional: Vec<UnitRecord>,
    pub lint_check_commands_evidence_only: bool,
}

/// Merge scan workspaces with units; descriptor-less roots become provisional units.
/// Dedupes by normalized path; fails on ambiguous stable unit ids.
pub fn discover_from_scan(
    scan: &ScanDocument,
    units: &UnitsDocument,
    freshness: Freshness,
) -> Result<ScanDiscovery, ControllerError> {
    let mut by_path: HashMap<String, UnitRecord> = HashMap::new();
    let mut id_paths: HashMap<String, Vec<String>> = HashMap::new();

    for unit in &units.units {
        let key = normalize_root(unit.path.as_deref().unwrap_or(&unit.id));
        if let Some(existing) = by_path.get(&key)
            && existing.id != unit.id
        {
            return Err(ControllerError::ambiguous(format!(
                "normalized root `{key}` maps to both `{}` and `{}`",
                existing.id, unit.id
            )));
        }
        id_paths
            .entry(unit.id.clone())
            .or_default()
            .push(key.clone());
        by_path.insert(key, unit.clone());
    }

    for (id, paths) in &id_paths {
        let unique: std::collections::BTreeSet<_> = paths.iter().collect();
        if unique.len() > 1 {
            return Err(ControllerError::ambiguous(format!(
                "unit id `{id}` appears under multiple roots: {}",
                paths.join(", ")
            )));
        }
    }

    let mut provisional = Vec::new();
    let mut workspaces = Vec::new();

    for ws in &scan.workspaces {
        let key = normalize_root(&ws.path);
        // lint_check_commands are evidence only — never execute
        let _ = &ws.lint_check_commands;
        workspaces.push(serde_json::json!({
            "path": ws.path,
            "kind": ws.kind,
            "native_manifests": ws.native_manifests,
            "lint_check_commands": ws.lint_check_commands,
            "descriptor_backed": by_path.contains_key(&key),
        }));

        if !by_path.contains_key(&key) {
            let id = provisional_id_from_path(&ws.path);
            let unit = UnitRecord {
                id: id.clone(),
                kind: ws.kind.clone().or(Some("workspace".into())),
                title: Some(format!("provisional:{id}")),
                status: Some("discovered".into()),
                domain: None,
                layer: None,
                stack: None,
                owner: None,
                runtime: None,
                path: Some(ws.path.clone()),
                native_manifests: ws.native_manifests.clone(),
                entrypoints: serde_json::json!({}),
                cli: None,
                provides: Vec::new(),
                requires: Vec::new(),
                policy: None,
                source: Some("scan-provisional".into()),
                provisional: true,
                routing_complete: Some(false),
            };
            provisional.push(unit);
        }
    }

    let units_out: Vec<_> = by_path.into_values().collect();
    Ok(ScanDiscovery {
        freshness,
        workspaces,
        units: units_out,
        provisional,
        lint_check_commands_evidence_only: true,
    })
}

fn normalize_root(path: &str) -> String {
    let p = Path::new(path);
    let mut parts = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
            std::path::Component::RootDir => parts.clear(),
            std::path::Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

fn provisional_id_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .replace([' ', '/'], "-")
}

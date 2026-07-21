//! Paths, read, freshness, and source-descriptor fallback.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::contracts::{RegistryGeneration, fingerprint_file};
use crate::error::ControllerError;

use super::types::{
    Freshness, RegistryFileKind, ScanDocument, ToolsDocument, UnitRecord, UnitsDocument,
};

/// Locations for Ontarch registry JSON and workspace root (for fingerprint path resolve).
#[derive(Debug, Clone)]
pub struct RegistryPaths {
    pub registry_root: PathBuf,
    pub workspace_root: PathBuf,
}

/// Resolve registry paths.
///
/// Precedence for registry root: `TAKOGAMI_ONTARCH_REGISTRY` → walk cwd for
/// `packages/ontarch/registry` → error.
/// Workspace root: `TAKOGAMI_WORKSPACE_ROOT` → inferred from registry path → cwd.
pub fn resolve_registry_paths() -> Result<RegistryPaths, ControllerError> {
    let registry_root = if let Ok(p) = std::env::var("TAKOGAMI_ONTARCH_REGISTRY") {
        PathBuf::from(p)
    } else {
        find_registry_from_cwd()?
    };
    let workspace_root = if let Ok(p) = std::env::var("TAKOGAMI_WORKSPACE_ROOT") {
        PathBuf::from(p)
    } else {
        infer_workspace_root(&registry_root)
    };
    Ok(RegistryPaths {
        registry_root,
        workspace_root,
    })
}

fn find_registry_from_cwd() -> Result<PathBuf, ControllerError> {
    let mut dir = std::env::current_dir()
        .map_err(|e| ControllerError::unavailable_source(format!("cannot resolve cwd: {e}")))?;
    loop {
        let candidate = dir.join("packages/ontarch/registry");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        let alt = dir.join("Build/src/workspaces/wfos/packages/ontarch/registry");
        if alt.is_dir() {
            return Ok(alt);
        }
        if !dir.pop() {
            break;
        }
    }
    Err(ControllerError::unavailable_source(
        "Ontarch registry not found; set TAKOGAMI_ONTARCH_REGISTRY or run from the wfos workspace",
    ))
}

fn infer_workspace_root(registry_root: &Path) -> PathBuf {
    // …/Workstreams/Build/src/workspaces/wfos/packages/ontarch/registry → Workstreams
    let mut p = registry_root.to_path_buf();
    for _ in 0..6 {
        if !p.pop() {
            break;
        }
    }
    if p.join("Build").is_dir() || p.join(".agents").is_dir() {
        return p;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Debug, Clone)]
pub struct RegistryAccess {
    pub paths: RegistryPaths,
}

impl RegistryAccess {
    pub fn new(paths: RegistryPaths) -> Self {
        Self { paths }
    }

    pub fn file_path(&self, kind: RegistryFileKind) -> PathBuf {
        self.paths.registry_root.join(kind.file_name())
    }

    /// Read units.json: miss returns empty doc + Miss; malformed → error; else freshness.
    pub fn load_units(&self) -> Result<(UnitsDocument, Freshness), ControllerError> {
        let path = self.file_path(RegistryFileKind::Units);
        if !path.exists() {
            return Ok((
                UnitsDocument {
                    generated_at: String::new(),
                    registry_generation: None,
                    summary: serde_json::json!({"total": 0}),
                    units: Vec::new(),
                },
                Freshness::Miss,
            ));
        }
        let text = fs::read_to_string(&path).map_err(|e| {
            ControllerError::invalid_registry(format!("cannot read {}: {e}", path.display()))
        })?;
        let doc: UnitsDocument = serde_json::from_str(&text)
            .map_err(|e| ControllerError::invalid_registry(format!("malformed units.json: {e}")))?;
        let freshness =
            evaluate_freshness(doc.registry_generation.as_ref(), &self.paths.workspace_root)?;
        Ok((doc, freshness))
    }

    pub fn load_scan(&self) -> Result<(ScanDocument, Freshness), ControllerError> {
        let path = self.file_path(RegistryFileKind::Scan);
        if !path.exists() {
            return Ok((
                ScanDocument {
                    generated_at: String::new(),
                    registry_generation: None,
                    root: None,
                    summary: serde_json::json!({}),
                    workspaces: Vec::new(),
                },
                Freshness::Miss,
            ));
        }
        let text = fs::read_to_string(&path).map_err(|e| {
            ControllerError::invalid_registry(format!("cannot read {}: {e}", path.display()))
        })?;
        let doc: ScanDocument = serde_json::from_str(&text)
            .map_err(|e| ControllerError::invalid_registry(format!("malformed scan.json: {e}")))?;
        let freshness =
            evaluate_freshness(doc.registry_generation.as_ref(), &self.paths.workspace_root)?;
        Ok((doc, freshness))
    }

    pub fn load_tools(&self) -> Result<(ToolsDocument, Freshness), ControllerError> {
        let path = self.file_path(RegistryFileKind::Tools);
        if !path.exists() {
            return Ok((
                ToolsDocument {
                    generated_at: String::new(),
                    summary: serde_json::json!({}),
                    tools: Vec::new(),
                },
                Freshness::Miss,
            ));
        }
        let text = fs::read_to_string(&path).map_err(|e| {
            ControllerError::invalid_registry(format!("cannot read {}: {e}", path.display()))
        })?;
        let doc: ToolsDocument = serde_json::from_str(&text)
            .map_err(|e| ControllerError::invalid_registry(format!("malformed tools.json: {e}")))?;
        Ok((doc, Freshness::Hit))
    }

    /// Source fallback: provisional units from `*.descriptor.toml` under configured roots.
    pub fn source_fallback_units(&self) -> Result<Vec<UnitRecord>, ControllerError> {
        let mut roots = Vec::new();
        let colocated = self
            .paths
            .workspace_root
            .join("Build/src/workspaces/wfos/packages/ontarch/descriptors");
        if colocated.is_dir() {
            roots.push(colocated);
        }
        if let Some(parent) = self.paths.registry_root.parent() {
            let central = parent.join("descriptors");
            if central.is_dir() && !roots.iter().any(|r| r == &central) {
                roots.push(central);
            }
        }
        let fixture = self.paths.registry_root.join("sources/descriptors");
        if fixture.is_dir() {
            roots.push(fixture);
        }

        let mut by_id: BTreeMap<String, UnitRecord> = BTreeMap::new();
        for root in roots {
            let Ok(entries) = fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(unit) = provisional_from_descriptor(&path) {
                    by_id.insert(unit.id.clone(), unit);
                }
            }
        }
        Ok(by_id.into_values().collect())
    }

    pub fn contracts_readable(&self) -> (bool, String) {
        let units = self.file_path(RegistryFileKind::Units);
        if units.exists() {
            match fs::read_to_string(&units).and_then(|t| {
                serde_json::from_str::<UnitsDocument>(&t)
                    .map(|_| ())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            }) {
                Ok(()) => (true, format!("units.json readable at {}", units.display())),
                Err(e) => (false, format!("units.json unreadable: {e}")),
            }
        } else {
            (false, format!("units.json missing at {}", units.display()))
        }
    }
}

/// Compare embedded fingerprints to current source bytes.
pub fn evaluate_freshness(
    generation: Option<&RegistryGeneration>,
    workspace_root: &Path,
) -> Result<Freshness, ControllerError> {
    let Some(meta) = generation else {
        return Ok(Freshness::Stale);
    };
    if meta.source_fingerprints.is_empty() {
        return Ok(Freshness::Stale);
    }
    for fp in &meta.source_fingerprints {
        let path = resolve_fingerprint_path(workspace_root, &fp.path);
        if !path.is_file() {
            return Ok(Freshness::Stale);
        }
        let current = fingerprint_file(&path, &fp.path).map_err(|e| {
            ControllerError::invalid_registry(format!("cannot fingerprint {}: {e}", path.display()))
        })?;
        if current.digest != fp.digest || current.algorithm != fp.algorithm {
            return Ok(Freshness::Stale);
        }
    }
    Ok(Freshness::Hit)
}

fn resolve_fingerprint_path(workspace_root: &Path, recorded: &str) -> PathBuf {
    let p = Path::new(recorded);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    workspace_root.join(recorded)
}

/// ponytail: TOML-subset top-level string extract for source fallback; ceiling = nested
/// tables / multiline; upgrade = toml crate when richer fields are required.
fn provisional_from_descriptor(path: &Path) -> Option<UnitRecord> {
    let text = fs::read_to_string(path).ok()?;
    let id = extract_toml_string(&text, "id")?;
    let kind = extract_toml_string(&text, "kind");
    let title = extract_toml_string(&text, "title");
    let status = extract_toml_string(&text, "status");
    let domain = extract_toml_string(&text, "domain");
    let layer = extract_toml_string(&text, "layer");
    Some(UnitRecord {
        id,
        kind,
        title,
        status,
        domain,
        layer,
        stack: None,
        owner: None,
        runtime: None,
        path: Some(path.display().to_string()),
        native_manifests: Vec::new(),
        entrypoints: serde_json::json!({}),
        cli: None,
        provides: Vec::new(),
        requires: Vec::new(),
        policy: None,
        source: Some("source-fallback".into()),
        provisional: true,
        routing_complete: Some(false),
    })
}

fn extract_toml_string(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let prefix = format!("{key} ");
        let prefix_eq = format!("{key}=");
        if let Some(rest) = line
            .strip_prefix(&prefix)
            .or_else(|| line.strip_prefix(&prefix_eq))
        {
            let rest = rest.trim().trim_start_matches('=').trim();
            if let Some(s) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
                return Some(s.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{SourceFingerprint, fingerprint_bytes};

    #[test]
    fn missing_generation_is_stale() {
        assert_eq!(
            evaluate_freshness(None, Path::new("/tmp")).unwrap(),
            Freshness::Stale
        );
    }

    #[test]
    fn matching_fingerprints_are_hit() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("a.toml");
        fs::write(&src, b"id = \"x\"\n").unwrap();
        let fp = fingerprint_file(&src, "a.toml").unwrap();
        let meta = RegistryGeneration {
            generated_at: "t".into(),
            source_fingerprints: vec![fp],
        };
        assert_eq!(
            evaluate_freshness(Some(&meta), temp.path()).unwrap(),
            Freshness::Hit
        );
    }

    #[test]
    fn mismatched_fingerprints_are_stale() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("a.toml");
        fs::write(&src, b"id = \"x\"\n").unwrap();
        let meta = RegistryGeneration {
            generated_at: "t".into(),
            source_fingerprints: vec![SourceFingerprint {
                path: "a.toml".into(),
                algorithm: "sha256".into(),
                digest: fingerprint_bytes(b"other").digest,
            }],
        };
        assert_eq!(
            evaluate_freshness(Some(&meta), temp.path()).unwrap(),
            Freshness::Stale
        );
    }
}

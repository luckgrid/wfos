//! Workspace-relative path normalization and manifest selection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use super::entrypoint::NormalizedEntrypoint;
use super::resolver::{BackendKind, ResolutionCode};
use crate::registry::UnitDefinition;

#[derive(Debug, Clone)]
pub struct ResolvedCwd {
    pub display: String,
    pub canonical: PathBuf,
}

pub fn resolve_cwd(
    workspace_root: &Path,
    unit: &UnitDefinition,
    entry: &NormalizedEntrypoint,
) -> Result<ResolvedCwd, ResolutionCode> {
    let raw = entry
        .cwd
        .as_deref()
        .or(unit.root.as_deref())
        .or(unit.path.as_deref())
        .ok_or_else(|| ResolutionCode::InvalidCwd {
            message: "no cwd from entrypoint or unit paths".into(),
        })?;

    let candidate = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        workspace_root.join(raw)
    };

    let canonical = candidate
        .canonicalize()
        .map_err(|e| ResolutionCode::InvalidCwd {
            message: format!("cwd `{raw}` cannot be resolved: {e}"),
        })?;

    if !canonical.is_dir() {
        return Err(ResolutionCode::InvalidCwd {
            message: format!("cwd `{}` is not a directory", canonical.display()),
        });
    }

    let root_canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    if !canonical.starts_with(&root_canon) {
        return Err(ResolutionCode::InvalidCwd {
            message: "cwd escapes configured workspace root".into(),
        });
    }

    let display = workspace_relative_display(&root_canon, &canonical);
    Ok(ResolvedCwd { display, canonical })
}

pub fn resolve_manifests(
    workspace_root: &Path,
    cwd: &Path,
    unit: &UnitDefinition,
    entry: &NormalizedEntrypoint,
    backend: BackendKind,
) -> Result<(Vec<String>, Vec<PathBuf>), ResolutionCode> {
    let selected: Vec<String> = if !entry.source_manifests.is_empty() {
        entry.source_manifests.clone()
    } else if backend == BackendKind::MoonTask {
        vec!["moon.yml".into()]
    } else if unit.native_manifests.len() == 1 {
        unit.native_manifests.clone()
    } else if unit.native_manifests.is_empty() {
        Vec::new()
    } else {
        let mut cands = unit.native_manifests.clone();
        cands.sort();
        return Err(ResolutionCode::ManifestAmbiguous { candidates: cands });
    };

    let root_canon =
        workspace_root
            .canonicalize()
            .map_err(|_| ResolutionCode::MissingManifest {
                message: "configured workspace root cannot be resolved".into(),
            })?;

    let mut declared = BTreeSet::new();
    for name in &unit.native_manifests {
        let canonical = canonical_manifest(workspace_root, cwd, &root_canon, name, "declared")?;
        declared.insert(canonical);
    }

    let mut selected_by_identity = BTreeMap::new();
    for name in &selected {
        let canonical = canonical_manifest(workspace_root, cwd, &root_canon, name, "selected")?;
        if !declared.is_empty() && !declared.contains(&canonical) {
            let display = workspace_relative_display(&root_canon, &canonical);
            return Err(ResolutionCode::MissingManifest {
                message: format!("manifest `{display}` is not authorized by the unit"),
            });
        }
        selected_by_identity.insert(
            canonical.clone(),
            workspace_relative_display(&root_canon, &canonical),
        );
    }

    if backend == BackendKind::MoonTask
        && !selected_by_identity
            .keys()
            .any(|path| path.file_name().and_then(|name| name.to_str()) == Some("moon.yml"))
    {
        return Err(ResolutionCode::MissingManifest {
            message: "Moon plan requires an authorized moon.yml".into(),
        });
    }

    let absolute = selected_by_identity.keys().cloned().collect();
    let display = selected_by_identity.into_values().collect();
    Ok((display, absolute))
}

fn canonical_manifest(
    workspace_root: &Path,
    cwd: &Path,
    root_canon: &Path,
    name: &str,
    role: &str,
) -> Result<PathBuf, ResolutionCode> {
    let path = resolve_manifest_path(workspace_root, cwd, name);
    let canonical = path
        .canonicalize()
        .map_err(|_| ResolutionCode::MissingManifest {
            message: format!("{role} manifest `{name}` not found"),
        })?;
    if !canonical.is_file() {
        return Err(ResolutionCode::MissingManifest {
            message: format!("{role} manifest `{name}` is not a regular file"),
        });
    }
    if !canonical.starts_with(root_canon) {
        return Err(ResolutionCode::MissingManifest {
            message: format!("{role} manifest escapes configured workspace root"),
        });
    }
    Ok(canonical)
}

fn resolve_manifest_path(workspace_root: &Path, cwd: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let from_cwd = cwd.join(name);
    if from_cwd.exists() {
        return from_cwd;
    }
    workspace_root.join(name)
}

pub fn workspace_relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(normalize_components)
        .unwrap_or_else(|_| path.display().to_string())
}

fn normalize_components(path: &Path) -> String {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ExecutionClass;

    fn unit(manifests: &[&str]) -> UnitDefinition {
        UnitDefinition {
            id: "demo".into(),
            path: Some("demo".into()),
            root: Some("demo".into()),
            native_manifests: manifests.iter().map(|value| (*value).into()).collect(),
            entrypoints: Default::default(),
            descriptor_path: "demo.descriptor.toml".into(),
            provisional: false,
            routing_complete: true,
        }
    }

    fn entry(manifests: &[&str]) -> NormalizedEntrypoint {
        NormalizedEntrypoint {
            program: "demo".into(),
            args: Vec::new(),
            cwd: Some("demo".into()),
            env_keys: Vec::new(),
            backend: Some("native".into()),
            adapter: Some("direct".into()),
            source_manifests: manifests.iter().map(|value| (*value).into()).collect(),
            required_policies: Vec::new(),
            execution_class: ExecutionClass::Direct,
            runtime_provider: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn manifest_authorization_uses_canonical_identity_not_basename() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("demo");
        std::fs::create_dir_all(cwd.join("a")).unwrap();
        std::fs::create_dir_all(cwd.join("b")).unwrap();
        std::fs::write(cwd.join("a/Cargo.toml"), "a").unwrap();
        std::fs::write(cwd.join("b/Cargo.toml"), "b").unwrap();

        let error = resolve_manifests(
            temp.path(),
            &cwd,
            &unit(&["a/Cargo.toml"]),
            &entry(&["b/Cargo.toml"]),
            BackendKind::NativeDirect,
        )
        .unwrap_err();

        assert!(matches!(&error, ResolutionCode::MissingManifest { .. }));
        assert!(error.message().contains("not authorized"));
    }

    #[test]
    fn duplicate_text_paths_canonicalize_to_one_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("demo");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("Cargo.toml"), "a").unwrap();

        let (display, canonical) = resolve_manifests(
            temp.path(),
            &cwd,
            &unit(&["Cargo.toml", "./Cargo.toml"]),
            &entry(&["Cargo.toml", "./Cargo.toml"]),
            BackendKind::NativeDirect,
        )
        .unwrap();

        assert_eq!(display, vec!["demo/Cargo.toml"]);
        assert_eq!(canonical.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn manifest_symlink_escape_fails_closed() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("demo");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(outside.path().join("Cargo.toml"), "outside").unwrap();
        symlink(outside.path().join("Cargo.toml"), cwd.join("Cargo.toml")).unwrap();

        let error = resolve_manifests(
            temp.path(),
            &cwd,
            &unit(&["Cargo.toml"]),
            &entry(&["Cargo.toml"]),
            BackendKind::NativeDirect,
        )
        .unwrap_err();

        assert!(matches!(&error, ResolutionCode::MissingManifest { .. }));
        assert!(
            error
                .message()
                .contains("escapes configured workspace root")
        );
    }
}

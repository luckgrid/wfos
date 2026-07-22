//! Workspace-relative path normalization and manifest selection.

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

    if backend == BackendKind::MoonTask && !selected.iter().any(|m| m.ends_with("moon.yml")) {
        return Err(ResolutionCode::MissingManifest {
            message: "Moon plan requires authoritative moon.yml".into(),
        });
    }

    let root_canon = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let mut display = Vec::new();
    let mut absolute = Vec::new();
    for name in &selected {
        let path = resolve_manifest_path(workspace_root, cwd, name);
        let canon = path
            .canonicalize()
            .map_err(|_| ResolutionCode::MissingManifest {
                message: format!("manifest `{name}` not found"),
            })?;
        if !canon.is_file() {
            return Err(ResolutionCode::MissingManifest {
                message: format!("manifest `{name}` is not a regular file"),
            });
        }
        if !canon.starts_with(&root_canon) {
            return Err(ResolutionCode::MissingManifest {
                message: format!("manifest `{name}` escapes workspace root"),
            });
        }
        // Declared by unit when unit lists manifests.
        if !unit.native_manifests.is_empty() {
            let base = Path::new(name)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(name);
            let declared = unit.native_manifests.iter().any(|m| {
                m == name || Path::new(m).file_name().and_then(|s| s.to_str()) == Some(base)
            });
            if !declared {
                return Err(ResolutionCode::MissingManifest {
                    message: format!("manifest `{name}` not declared by unit"),
                });
            }
        }
        display.push(name.clone());
        absolute.push(canon);
    }
    display.sort();
    Ok((display, absolute))
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

//! Filesystem-only executable lookup (no which/shell/process).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::explain::{ExecutableProvenance, ExecutableSelectionSource};
use super::paths::workspace_relative_display;
use super::resolver::{BackendKind, ResolutionCode};
use crate::registry::ToolRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutable {
    pub canonical: PathBuf,
    pub provenance: ExecutableProvenance,
}

pub trait ExecutableLocator {
    fn locate(
        &self,
        program: &str,
        cwd: &Path,
        path_dirs: &[PathBuf],
        backend: BackendKind,
        tools: &[ToolRecord],
        workspace_root: &Path,
    ) -> Result<ResolvedExecutable, ResolutionCode>;
}

#[derive(Debug, Default)]
pub struct FilesystemLocator;

impl ExecutableLocator for FilesystemLocator {
    fn locate(
        &self,
        program: &str,
        cwd: &Path,
        path_dirs: &[PathBuf],
        backend: BackendKind,
        tools: &[ToolRecord],
        workspace_root: &Path,
    ) -> Result<ResolvedExecutable, ResolutionCode> {
        locate_executable(program, cwd, path_dirs, backend, tools, workspace_root)
    }
}

pub fn locate_executable(
    program: &str,
    cwd: &Path,
    path_dirs: &[PathBuf],
    backend: BackendKind,
    tools: &[ToolRecord],
    workspace_root: &Path,
) -> Result<ResolvedExecutable, ResolutionCode> {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let prog = Path::new(program);
    if prog.is_absolute() {
        return accept_absolute(prog, tools, &root);
    }
    if program.contains('/') || program.contains('\\') {
        let canonical =
            accept_file(&cwd.join(program)).ok_or_else(|| ResolutionCode::MissingExecutable {
                message: format!("relative program `{program}` not found under cwd"),
            })?;
        if !canonical.starts_with(&root) {
            return Err(ResolutionCode::MissingExecutable {
                message: "relative program escapes configured workspace root".into(),
            });
        }
        return Ok(ResolvedExecutable {
            provenance: workspace_provenance(&root, &canonical),
            canonical,
        });
    }

    if backend == BackendKind::PanoplyTool {
        return locate_panoply(program, path_dirs, tools, &root);
    }

    locate_on_ordered_path(program, path_dirs, &root, None)
}

fn locate_panoply(
    program: &str,
    path_dirs: &[PathBuf],
    tools: &[ToolRecord],
    workspace_root: &Path,
) -> Result<ResolvedExecutable, ResolutionCode> {
    let mut claims: Vec<&ToolRecord> = tools
        .iter()
        .filter(|tool| {
            tool.installed == Some(true)
                && (tool.id == program
                    || tool
                        .detect
                        .as_deref()
                        .and_then(|detect| Path::new(detect).file_name())
                        .and_then(|name| name.to_str())
                        == Some(program))
        })
        .collect();
    claims.sort_by(|a, b| a.id.cmp(&b.id).then(a.detect.cmp(&b.detect)));

    if claims.is_empty() {
        return Err(ResolutionCode::MissingExecutable {
            message: format!("Panoply has no installed tool projection for `{program}`"),
        });
    }

    let mut detected: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for tool in &claims {
        let Some(detect) = tool.detect.as_deref() else {
            continue;
        };
        let path = Path::new(detect);
        if !path.is_absolute() {
            continue;
        }
        if let Some(canonical) = accept_file(path) {
            detected
                .entry(canonical)
                .or_default()
                .insert(tool.id.clone());
        }
    }

    match detected.len() {
        1 => {
            let (canonical, ids) = detected.into_iter().next().expect("one candidate");
            let tool_id = ids.into_iter().next();
            let display_path = safe_display_path(workspace_root, &canonical);
            return Ok(ResolvedExecutable {
                canonical,
                provenance: ExecutableProvenance {
                    selection_source: ExecutableSelectionSource::PanoplyDetect,
                    tool_id,
                    path_index: None,
                    display_path,
                },
            });
        }
        count if count > 1 => {
            let candidates = detected
                .into_iter()
                .enumerate()
                .map(|(index, (path, ids))| {
                    if let Some(display) = safe_display_path(workspace_root, &path) {
                        format!(
                            "{} ({display})",
                            ids.into_iter().collect::<Vec<_>>().join("+")
                        )
                    } else {
                        format!(
                            "{} (external-candidate-{})",
                            ids.into_iter().collect::<Vec<_>>().join("+"),
                            index + 1
                        )
                    }
                })
                .collect();
            return Err(ResolutionCode::ExecutableAmbiguous { candidates });
        }
        _ => {}
    }

    let claim_ids: BTreeSet<String> = claims.iter().map(|tool| tool.id.clone()).collect();
    if claim_ids.len() != 1 {
        return Err(ResolutionCode::ExecutableAmbiguous {
            candidates: claim_ids.into_iter().collect(),
        });
    }
    let tool_id = claim_ids.into_iter().next();
    locate_on_ordered_path(program, path_dirs, workspace_root, tool_id)
}

fn locate_on_ordered_path(
    program: &str,
    path_dirs: &[PathBuf],
    workspace_root: &Path,
    tool_id: Option<String>,
) -> Result<ResolvedExecutable, ResolutionCode> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for dir in path_dirs {
        let Ok(canonical) = dir.canonicalize() else {
            continue;
        };
        if canonical.is_dir() && seen.insert(canonical.clone()) {
            ordered.push(canonical);
        }
    }

    for (path_index, dir) in ordered.iter().enumerate() {
        if let Some(canonical) = accept_file(&dir.join(program)) {
            let display_path = safe_display_path(workspace_root, &canonical);
            return Ok(ResolvedExecutable {
                canonical,
                provenance: ExecutableProvenance {
                    selection_source: ExecutableSelectionSource::Path,
                    tool_id,
                    path_index: Some(path_index),
                    display_path,
                },
            });
        }
    }
    Err(ResolutionCode::MissingExecutable {
        message: format!("executable `{program}` not found on PATH snapshot"),
    })
}

fn accept_absolute(
    path: &Path,
    tools: &[ToolRecord],
    workspace_root: &Path,
) -> Result<ResolvedExecutable, ResolutionCode> {
    let canonical = accept_file(path).ok_or_else(|| ResolutionCode::MissingExecutable {
        message: "absolute program is not an executable file".into(),
    })?;

    if canonical.starts_with(workspace_root) {
        return Ok(ResolvedExecutable {
            provenance: workspace_provenance(workspace_root, &canonical),
            canonical,
        });
    }

    let mut matching_ids = BTreeSet::new();
    for tool in tools.iter().filter(|tool| tool.installed == Some(true)) {
        let Some(detect) = tool.detect.as_deref() else {
            continue;
        };
        let detect_path = Path::new(detect);
        if detect_path.is_absolute()
            && accept_file(detect_path).as_deref() == Some(canonical.as_path())
        {
            matching_ids.insert(tool.id.clone());
        }
    }
    if let Some(tool_id) = matching_ids.into_iter().next() {
        return Ok(ResolvedExecutable {
            canonical,
            provenance: ExecutableProvenance {
                selection_source: ExecutableSelectionSource::PanoplyDetect,
                tool_id: Some(tool_id),
                path_index: None,
                display_path: None,
            },
        });
    }

    Err(ResolutionCode::MissingExecutable {
        message: "absolute program outside workspace and not a Panoply detect path".into(),
    })
}

fn accept_file(path: &Path) -> Option<PathBuf> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    path.canonicalize().ok()
}

fn workspace_provenance(workspace_root: &Path, canonical: &Path) -> ExecutableProvenance {
    ExecutableProvenance {
        selection_source: ExecutableSelectionSource::WorkspaceRelative,
        tool_id: None,
        path_index: None,
        display_path: safe_display_path(workspace_root, canonical),
    }
}

fn safe_display_path(workspace_root: &Path, canonical: &Path) -> Option<String> {
    canonical
        .starts_with(workspace_root)
        .then(|| workspace_relative_display(workspace_root, canonical))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn executable(path: &Path) {
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn ordered_path_deduplicates_directories_and_selects_first_hit() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        executable(&first.join("demo"));
        executable(&second.join("demo"));

        let found = locate_executable(
            "demo",
            temp.path(),
            &[first.clone(), first, second],
            BackendKind::NativeDirect,
            &[],
            temp.path(),
        )
        .unwrap();

        assert_eq!(
            found.canonical,
            fs::canonicalize(temp.path().join("first/demo")).unwrap()
        );
        assert_eq!(found.provenance.path_index, Some(0));
    }

    #[test]
    fn distinct_panoply_detect_paths_are_ambiguous_independent_of_input_order() {
        let temp = tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        executable(&a);
        executable(&b);
        let tool = |id: &str, path: &Path| ToolRecord {
            id: id.into(),
            module: None,
            installed: Some(true),
            default: None,
            agent_safe: None,
            detect: Some(path.display().to_string()),
            version: None,
        };
        let first = vec![tool("demo", &a), tool("demo", &b)];
        let second = vec![tool("demo", &b), tool("demo", &a)];

        let a_err = locate_executable(
            "demo",
            temp.path(),
            &[],
            BackendKind::PanoplyTool,
            &first,
            temp.path(),
        )
        .unwrap_err();
        let b_err = locate_executable(
            "demo",
            temp.path(),
            &[],
            BackendKind::PanoplyTool,
            &second,
            temp.path(),
        )
        .unwrap_err();
        assert_eq!(a_err.message(), b_err.message());
        assert!(matches!(a_err, ResolutionCode::ExecutableAmbiguous { .. }));
    }

    #[test]
    fn duplicate_panoply_rows_for_same_canonical_file_deduplicate() {
        let temp = tempdir().unwrap();
        let program = temp.path().join("demo");
        executable(&program);
        let row = || ToolRecord {
            id: "demo".into(),
            module: None,
            installed: Some(true),
            default: None,
            agent_safe: None,
            detect: Some(program.display().to_string()),
            version: None,
        };

        let found = locate_executable(
            "demo",
            temp.path(),
            &[],
            BackendKind::PanoplyTool,
            &[row(), row()],
            temp.path(),
        )
        .unwrap();

        assert_eq!(found.canonical, program.canonicalize().unwrap());
        assert_eq!(
            found.provenance.selection_source,
            ExecutableSelectionSource::PanoplyDetect
        );
    }
}

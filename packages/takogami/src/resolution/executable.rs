//! Filesystem-only executable lookup (no which/shell/process).

use std::fs;
use std::path::{Path, PathBuf};

use super::resolver::{BackendKind, ResolutionCode};
use crate::registry::ToolRecord;

pub trait ExecutableLocator {
    fn locate(
        &self,
        program: &str,
        cwd: &Path,
        path_dirs: &[PathBuf],
        backend: BackendKind,
        tools: &[ToolRecord],
        workspace_root: &Path,
    ) -> Result<PathBuf, ResolutionCode>;
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
    ) -> Result<PathBuf, ResolutionCode> {
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
) -> Result<PathBuf, ResolutionCode> {
    let prog = Path::new(program);
    if prog.is_absolute() {
        return accept_absolute(prog, tools, workspace_root);
    }
    if program.contains('/') || program.contains('\\') {
        let candidate = cwd.join(program);
        return accept_file(&candidate).ok_or_else(|| ResolutionCode::MissingExecutable {
            message: format!("relative program `{program}` not found under cwd"),
        });
    }

    // Panoply detect path takes precedence when it matches.
    if backend == BackendKind::PanoplyTool
        && let Some(tool) = matching_tool(program, tools)
        && let Some(detect) = tool.detect.as_deref()
    {
        let p = Path::new(detect);
        if p.is_absolute()
            && let Some(ok) = accept_file(p)
        {
            return Ok(ok);
        }
    }

    // ponytail: first PATH hit wins (which-compatible). Ceiling: proto may list both
    // shim and real binaries; upgrade = prefer Panoply detect path / content equality.
    for dir in path_dirs {
        let candidate = dir.join(program);
        if let Some(ok) = accept_file(&candidate) {
            return Ok(ok);
        }
    }
    Err(ResolutionCode::MissingExecutable {
        message: format!("executable `{program}` not found on PATH snapshot"),
    })
}

fn matching_tool<'a>(program: &str, tools: &'a [ToolRecord]) -> Option<&'a ToolRecord> {
    tools.iter().find(|t| {
        t.installed == Some(true)
            && (t.id == program
                || t.detect
                    .as_deref()
                    .and_then(|d| Path::new(d).file_name())
                    .and_then(|n| n.to_str())
                    == Some(program))
    })
}

fn accept_absolute(
    path: &Path,
    tools: &[ToolRecord],
    workspace_root: &Path,
) -> Result<PathBuf, ResolutionCode> {
    let panoply_match = tools
        .iter()
        .any(|t| t.installed == Some(true) && t.detect.as_deref().map(Path::new) == Some(path));
    let file = accept_file(path).ok_or_else(|| ResolutionCode::MissingExecutable {
        message: format!(
            "absolute program `{}` not an executable file",
            path.display()
        ),
    })?;
    if panoply_match {
        return Ok(file);
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let canon = file.canonicalize().unwrap_or(file.clone());
    if canon.starts_with(&root) {
        Ok(file)
    } else {
        Err(ResolutionCode::MissingExecutable {
            message: "absolute program outside workspace and not a Panoply detect path".into(),
        })
    }
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
    Some(path.to_path_buf())
}

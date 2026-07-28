//! Typed post-authorization projection mutations for in-process tests only.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::process;
use super::{ExecutionOptions, ExecutionReport, ProjectionExecutor};
use crate::policy::AuthorizedProjectionPlan;
use crate::projection::SealedProjectionPlan;

const COMMON_SH_LOGICAL: &str = "packages/ontarch/lib/common.sh";

/// Post-seal mutation applied immediately before production preflight (tests only).
pub(crate) struct ProjectionTestMutation {
    root: PathBuf,
    kind: MutationKind,
}

#[derive(Debug, Clone)]
pub(crate) enum MutationKind {
    RemoveExecutable,
    RenameCwd,
    RewriteSource,
    RemoveSource,
    SourceSymlink,
    SourceDanglingSymlink,
    SourceFifo,
    SourceDirectory,
    SourceSameLengthDrift,
    RewriteHelper {
        name: String,
    },
    #[allow(dead_code)]
    // reserved for extended helper drift matrix; RewriteHelper covers content drift
    RemoveHelper {
        name: String,
    },
    #[allow(dead_code)]
    HelperSymlinkReplacement {
        name: String,
        link_target: PathBuf,
    },
    InsertHelperShadow {
        dir: PathBuf,
        name: String,
        shadow: HelperShadowSpec,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum HelperShadowSpec {
    DistinctBytes,
    SameBytesAs(PathBuf),
    WorldWritable,
    SymlinkTo(PathBuf),
}

impl ProjectionTestMutation {
    pub(crate) fn new(root: PathBuf, kind: MutationKind) -> Self {
        Self { root, kind }
    }

    pub(crate) fn apply(&self, plan: &SealedProjectionPlan) {
        match &self.kind {
            MutationKind::RemoveExecutable => {
                let p = plan.executable_path();
                self.assert_under_root(p);
                let _ = fs::remove_file(p);
            }
            MutationKind::RenameCwd => {
                let cwd = plan.cwd_path();
                self.assert_under_root(cwd);
                let bogey = cwd.with_extension("drifted");
                self.assert_under_root(&bogey);
                let _ = fs::rename(cwd, &bogey);
            }
            MutationKind::RewriteSource => {
                let p = self.common_source(plan);
                let _ = fs::write(p, b"# drifted\n");
            }
            MutationKind::RemoveSource => {
                let p = self.common_source(plan);
                let _ = fs::remove_file(p);
            }
            MutationKind::SourceSymlink => {
                let common = self.common_source(plan);
                let backup = common.with_extension("bak");
                self.assert_under_root(&backup);
                let _ = fs::rename(common, &backup);
                let _ = std::os::unix::fs::symlink(&backup, common);
            }
            MutationKind::SourceDanglingSymlink => {
                let common = self.common_source(plan);
                let _ = fs::remove_file(common);
                let missing = common.with_extension("missing");
                self.assert_under_root(&missing);
                let _ = std::os::unix::fs::symlink(&missing, common);
            }
            MutationKind::SourceFifo => {
                let common = self.common_source(plan);
                let _ = fs::remove_file(common);
                let _ = std::process::Command::new("/usr/bin/mkfifo")
                    .arg(common)
                    .status();
            }
            MutationKind::SourceDirectory => {
                let common = self.common_source(plan);
                let _ = fs::remove_file(common);
                let _ = fs::create_dir(common);
            }
            MutationKind::SourceSameLengthDrift => {
                let common = self.common_source(plan);
                let original = fs::read(common).unwrap_or_else(|_| b"#\n".to_vec());
                let mut replacement = vec![b'X'; original.len()];
                if let Some(last) = replacement.last_mut() {
                    *last = b'\n';
                }
                let _ = fs::write(common, replacement);
            }
            MutationKind::RewriteHelper { name } => {
                let p = self.helper_canonical(plan, name);
                let _ = fs::write(p, b"#!/bin/sh\necho drifted-helper\n");
                if let Ok(meta) = fs::metadata(p) {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o755);
                    let _ = fs::set_permissions(p, perms);
                }
            }
            MutationKind::RemoveHelper { name } => {
                let p = self.helper_canonical(plan, name);
                let _ = fs::remove_file(p);
            }
            MutationKind::HelperSymlinkReplacement { name, link_target } => {
                let p = self.helper_canonical(plan, name);
                self.assert_under_root(link_target);
                let _ = fs::remove_file(p);
                let _ = std::os::unix::fs::symlink(link_target, p);
            }
            MutationKind::InsertHelperShadow { dir, name, shadow } => {
                self.assert_under_root(dir);
                let target = dir.join(name);
                self.assert_under_root(&target);
                match shadow {
                    HelperShadowSpec::SymlinkTo(link_to) => {
                        self.assert_under_root(link_to);
                        let _ = std::os::unix::fs::symlink(link_to, &target);
                        return;
                    }
                    HelperShadowSpec::DistinctBytes => {
                        let _ = fs::write(&target, b"#!/bin/sh\necho shadow\n");
                    }
                    HelperShadowSpec::SameBytesAs(src) => {
                        self.assert_under_root(src);
                        let bytes =
                            fs::read(src).unwrap_or_else(|_| b"#!/bin/sh\necho shadow\n".to_vec());
                        let _ = fs::write(&target, bytes);
                    }
                    HelperShadowSpec::WorldWritable => {
                        let _ = fs::write(&target, b"#!/bin/sh\necho shadow\n");
                    }
                }
                if let Ok(meta) = fs::metadata(&target) {
                    let mut perms = meta.permissions();
                    let mode = match shadow {
                        HelperShadowSpec::WorldWritable => 0o757,
                        _ => 0o755,
                    };
                    perms.set_mode(mode);
                    let _ = fs::set_permissions(&target, perms);
                }
            }
        }
    }

    fn common_source<'a>(&self, plan: &'a SealedProjectionPlan) -> &'a Path {
        let path = plan
            .source_abs_paths()
            .iter()
            .zip(plan.source_fingerprints().iter())
            .find(|(_, fp)| fp.path == COMMON_SH_LOGICAL)
            .map(|(abs, _)| abs.as_path())
            .expect("common.sh bound source");
        self.assert_under_root(path);
        path
    }

    fn helper_canonical<'a>(&self, plan: &'a SealedProjectionPlan, name: &str) -> &'a Path {
        let helper = plan
            .helper_identities()
            .iter()
            .find(|h| h.name == name)
            .unwrap_or_else(|| panic!("helper {name} not in sealed plan"));
        self.assert_under_root(&helper.canonical_path);
        helper.canonical_path.as_path()
    }

    fn assert_under_root(&self, target: &Path) {
        let root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let resolved = if target.exists() {
            target
                .canonicalize()
                .unwrap_or_else(|_| target.to_path_buf())
        } else if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
            parent
                .canonicalize()
                .map(|p| target.file_name().map(|n| p.join(n)).unwrap_or(p))
                .unwrap_or_else(|_| target.to_path_buf())
        } else {
            target.to_path_buf()
        };
        let resolved = if resolved.exists() {
            resolved.canonicalize().unwrap_or(resolved)
        } else {
            resolved
        };
        assert!(
            resolved.starts_with(&root),
            "mutation target {:?} must stay under fixture root {:?}",
            resolved,
            root
        );
    }
}

/// Applies a typed mutation at executor entry, then runs the production projection path.
pub(crate) struct MutatingProjectionExecutor {
    mutation: ProjectionTestMutation,
}

impl MutatingProjectionExecutor {
    pub(crate) fn new(mutation: ProjectionTestMutation) -> Self {
        Self { mutation }
    }
}

#[async_trait]
impl ProjectionExecutor for MutatingProjectionExecutor {
    async fn execute_projection(
        &self,
        authorized: &AuthorizedProjectionPlan,
        options: &ExecutionOptions,
    ) -> ExecutionReport {
        self.mutation.apply(authorized.plan());
        process::run_projection(authorized, options).await
    }
}

//! Sealed non-lifecycle projection plan and package-owned Ontarch resolution.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::op::ProjectionOperation;
use super::scope::ValidatedBinScope;
use crate::contracts::types::RequestRecord;
use crate::contracts::{SourceFingerprint, fingerprint_regular_file_nofollow};

const DIGEST_PAYLOAD_VERSION: &str = "s7-projection-v1";

/// Provider-neutral sealed projection plan. No public constructor.
#[derive(Debug, Clone)]
pub(crate) struct SealedProjectionPlan {
    operation: ProjectionOperation,
    executable_path: PathBuf,
    cwd_path: PathBuf,
    argv: Vec<String>,
    fixed_env: BTreeMap<String, String>,
    inherited_env_keys: Vec<String>,
    source_fingerprints: Vec<SourceFingerprint>,
    /// Absolute paths parallel to `source_fingerprints` for pre-spawn rehash (never logged).
    source_abs_paths: Vec<PathBuf>,
    safe_request: RequestRecord,
    safe_scope: Option<ValidatedBinScope>,
    session_id: String,
    profile_id: String,
    policy_ids: Vec<String>,
    plan_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectionSealError {
    OntarchMissing,
    OntarchNotFile,
    OntarchSymlink,
    OntarchNotExecutable,
    CwdMissing,
    CwdNotDir,
    Fingerprint(String),
    Identity(String),
    ToolPath(String),
}

impl ProjectionSealError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::OntarchMissing
            | Self::OntarchNotFile
            | Self::OntarchSymlink
            | Self::OntarchNotExecutable => "projection_tool_unavailable",
            Self::CwdMissing | Self::CwdNotDir => "projection_contract_changed",
            Self::Fingerprint(_) => "projection_contract_changed",
            Self::Identity(_) => "projection_contract_changed",
            Self::ToolPath(_) => "execution_contract",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::OntarchMissing => "canonical package Ontarch executable not found".into(),
            Self::OntarchNotFile => "canonical Ontarch path is not a regular file".into(),
            Self::OntarchSymlink => "canonical Ontarch executable must not be a symlink".into(),
            Self::OntarchNotExecutable => "canonical Ontarch is not executable".into(),
            Self::CwdMissing => "sealed workspace cwd missing".into(),
            Self::CwdNotDir => "sealed workspace cwd is not a directory".into(),
            Self::Fingerprint(m) | Self::Identity(m) | Self::ToolPath(m) => m.clone(),
        }
    }
}

impl SealedProjectionPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seal(
        operation: ProjectionOperation,
        registry_root: &Path,
        workspace_root: &Path,
        scope: Option<ValidatedBinScope>,
        session_id: String,
        profile_id: String,
        policy_ids: Vec<String>,
    ) -> Result<Self, ProjectionSealError> {
        let ontarch_pkg = registry_root
            .parent()
            .ok_or(ProjectionSealError::OntarchMissing)?;
        let executable = resolve_and_seal_ontarch(ontarch_pkg)?;
        let cwd = seal_cwd(workspace_root)?;

        let argv = operation.child_argv(scope.as_ref());

        let mut fixed_env = BTreeMap::new();
        fixed_env.insert("PANOPLY_AGENT".into(), "1".into());
        fixed_env.insert("NO_COLOR".into(), "1".into());
        fixed_env.insert(
            "WS_ROOT".into(),
            workspace_root.to_string_lossy().into_owned(),
        );
        let path = build_controller_tool_path()?;
        crate::execution::validate_controller_path(&path).map_err(ProjectionSealError::ToolPath)?;
        fixed_env.insert("PATH".into(), path);

        let inherited_env_keys: Vec<String> = Vec::new();
        let (source_fingerprints, source_abs_paths) =
            fingerprint_projection_sources(operation, ontarch_pkg)?;

        let mut flags = Vec::new();
        if let Some(mode) = operation.mode_flag() {
            flags.push(mode.to_string());
        }
        if scope.is_some() {
            flags.push("scope_provided".into());
        }
        let safe_request = RequestRecord {
            command: operation.request_command_name().into(),
            unit_id: None,
            verb: None,
            flags,
        };

        let mut policy_ids = policy_ids;
        policy_ids.sort();
        policy_ids.dedup();

        let plan_digest = compute_projection_digest(
            operation,
            &executable,
            &cwd,
            &argv,
            &fixed_env,
            &inherited_env_keys,
            &source_fingerprints,
            &safe_request,
            scope.as_ref().map(ValidatedBinScope::as_str),
            &session_id,
            &profile_id,
            &policy_ids,
        );

        Ok(Self {
            operation,
            executable_path: executable,
            cwd_path: cwd,
            argv,
            fixed_env,
            inherited_env_keys,
            source_fingerprints,
            source_abs_paths,
            safe_request,
            safe_scope: scope,
            session_id,
            profile_id,
            policy_ids,
            plan_digest,
        })
    }

    pub(crate) fn operation(&self) -> ProjectionOperation {
        self.operation
    }
    pub(crate) fn executable_path(&self) -> &Path {
        &self.executable_path
    }
    pub(crate) fn cwd_path(&self) -> &Path {
        &self.cwd_path
    }
    pub(crate) fn argv(&self) -> &[String] {
        &self.argv
    }
    pub(crate) fn fixed_env(&self) -> &BTreeMap<String, String> {
        &self.fixed_env
    }
    pub(crate) fn inherited_env_keys(&self) -> &[String] {
        &self.inherited_env_keys
    }
    pub(crate) fn source_fingerprints(&self) -> &[SourceFingerprint] {
        &self.source_fingerprints
    }
    pub(crate) fn safe_request(&self) -> &RequestRecord {
        &self.safe_request
    }
    pub(crate) fn safe_scope(&self) -> Option<&ValidatedBinScope> {
        self.safe_scope.as_ref()
    }
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }
    pub(crate) fn profile_id(&self) -> &str {
        &self.profile_id
    }
    pub(crate) fn policy_ids(&self) -> &[String] {
        &self.policy_ids
    }
    pub(crate) fn plan_digest(&self) -> &str {
        &self.plan_digest
    }

    /// Re-validate sealed identities immediately before spawn.
    pub(crate) fn preflight_identities(&self) -> Result<(), ProjectionSealError> {
        preflight_path_identity(&self.executable_path, true)?;
        preflight_path_identity(&self.cwd_path, false)?;
        Ok(())
    }

    /// Re-open and re-hash every bound source immediately before spawn.
    pub(crate) fn preflight_sources(&self) -> Result<(), ProjectionSealError> {
        if self.source_fingerprints.len() != self.source_abs_paths.len() {
            return Err(ProjectionSealError::Fingerprint(
                "source fingerprint set length mismatch".into(),
            ));
        }
        for (fp, abs) in self
            .source_fingerprints
            .iter()
            .zip(self.source_abs_paths.iter())
        {
            let fresh = fingerprint_regular_file_nofollow(abs, &fp.path).map_err(|e| {
                ProjectionSealError::Fingerprint(format!("source preflight {}: {e}", fp.path))
            })?;
            if fresh.digest != fp.digest || fresh.algorithm != fp.algorithm {
                return Err(ProjectionSealError::Fingerprint(format!(
                    "source digest drifted: {}",
                    fp.path
                )));
            }
        }
        Ok(())
    }
}

/// Build a controller-owned PATH from closed absolute platform directories.
/// Does not read or inherit the caller's PATH.
pub(crate) fn build_controller_tool_path() -> Result<String, ProjectionSealError> {
    let candidates: &[&str] = &[
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ];
    let mut dirs: Vec<String> = Vec::new();
    for cand in candidates {
        let p = Path::new(cand);
        if !p.is_absolute() {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            // Follow only to confirm directory existence; store the absolute candidate string.
            let Ok(meta2) = fs::metadata(p) else {
                continue;
            };
            if !meta2.is_dir() {
                continue;
            }
        } else if !meta.is_dir() {
            continue;
        }
        if !dirs.iter().any(|d| d == cand) {
            dirs.push((*cand).to_string());
        }
    }
    if dirs.is_empty() {
        return Err(ProjectionSealError::ToolPath(
            "no approved absolute tool directories available".into(),
        ));
    }
    Ok(dirs.join(":"))
}

fn resolve_and_seal_ontarch(ontarch_pkg: &Path) -> Result<PathBuf, ProjectionSealError> {
    let path = ontarch_pkg.join("bin/ontarch");
    if !path.exists() {
        return Err(ProjectionSealError::OntarchMissing);
    }
    let meta = fs::symlink_metadata(&path).map_err(|_| ProjectionSealError::OntarchMissing)?;
    if meta.file_type().is_symlink() {
        return Err(ProjectionSealError::OntarchSymlink);
    }
    if !meta.is_file() {
        return Err(ProjectionSealError::OntarchNotFile);
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(ProjectionSealError::OntarchNotExecutable);
    }
    path.canonicalize()
        .map_err(|e| ProjectionSealError::Identity(format!("canonicalize ontarch: {e}")))
}

fn seal_cwd(workspace_root: &Path) -> Result<PathBuf, ProjectionSealError> {
    if !workspace_root.exists() {
        return Err(ProjectionSealError::CwdMissing);
    }
    let meta = fs::metadata(workspace_root).map_err(|_| ProjectionSealError::CwdMissing)?;
    if !meta.is_dir() {
        return Err(ProjectionSealError::CwdNotDir);
    }
    workspace_root
        .canonicalize()
        .map_err(|e| ProjectionSealError::Identity(format!("canonicalize cwd: {e}")))
}

fn preflight_path_identity(path: &Path, must_be_file: bool) -> Result<(), ProjectionSealError> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| ProjectionSealError::Identity("sealed path missing before spawn".into()))?;
    if must_be_file {
        if meta.file_type().is_symlink() {
            return Err(ProjectionSealError::OntarchSymlink);
        }
        if !meta.is_file() {
            return Err(ProjectionSealError::OntarchNotFile);
        }
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(ProjectionSealError::OntarchNotExecutable);
        }
    } else if !meta.is_dir() {
        return Err(ProjectionSealError::CwdNotDir);
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| ProjectionSealError::Identity(format!("identity drift canonicalize: {e}")))?;
    if canonical != path {
        return Err(ProjectionSealError::Identity(
            "sealed path canonical identity drifted".into(),
        ));
    }
    Ok(())
}

fn required_source_rels(operation: ProjectionOperation) -> &'static [&'static str] {
    match operation {
        ProjectionOperation::BinReport => &[
            "bin/ontarch",
            "bin/ontarch-bin-report",
            "lib/common.sh",
            "lib/registry.sh",
            "policies/takogami.agent.policy.toml",
            "policies/agent-bin.policy.toml",
            "schemas/bin-inventory.schema.json",
        ],
        ProjectionOperation::BinCleanupReportOnly
        | ProjectionOperation::BinCleanupDryRun
        | ProjectionOperation::BinCleanupArchive
        | ProjectionOperation::BinCleanupDeleteApproved => &[
            "bin/ontarch",
            "bin/ontarch-bin-report",
            "bin/ontarch-bin-cleanup",
            "lib/common.sh",
            "lib/registry.sh",
            "policies/takogami.agent.policy.toml",
            "policies/agent-bin.policy.toml",
            "schemas/bin-inventory.schema.json",
            "schemas/bin-cleanup-plan.schema.json",
        ],
    }
}

fn fingerprint_projection_sources(
    operation: ProjectionOperation,
    ontarch_pkg: &Path,
) -> Result<(Vec<SourceFingerprint>, Vec<PathBuf>), ProjectionSealError> {
    let mut out = Vec::new();
    let mut abs_paths = Vec::new();
    for rel in required_source_rels(operation) {
        let abs = ontarch_pkg.join(rel);
        let display = format!("packages/ontarch/{rel}");
        let fp = fingerprint_regular_file_nofollow(&abs, &display).map_err(|e| {
            ProjectionSealError::Fingerprint(format!("required source {display}: {e}"))
        })?;
        out.push(fp);
        abs_paths.push(abs);
    }
    // Stable order for digest (already manifest order); keep abs parallel.
    Ok((out, abs_paths))
}

#[allow(clippy::too_many_arguments)]
fn compute_projection_digest(
    operation: ProjectionOperation,
    executable: &Path,
    cwd: &Path,
    argv: &[String],
    fixed_env: &BTreeMap<String, String>,
    inherited_env_keys: &[String],
    source_fingerprints: &[SourceFingerprint],
    safe_request: &RequestRecord,
    scope: Option<&str>,
    session_id: &str,
    profile_id: &str,
    policy_ids: &[String],
) -> String {
    #[derive(Serialize)]
    struct DigestPayload<'a> {
        version: &'static str,
        operation: &'static str,
        canonical_executable: PathIdentity,
        canonical_cwd: PathIdentity,
        argv: &'a [String],
        fixed_env: &'a BTreeMap<String, String>,
        inherited_env_keys: &'a [String],
        source_fingerprints: &'a [SourceFingerprint],
        safe_request: &'a RequestRecord,
        scope: Option<&'a str>,
        session_id: &'a str,
        profile_id: &'a str,
        policy_ids: &'a [String],
    }
    let payload = DigestPayload {
        version: DIGEST_PAYLOAD_VERSION,
        operation: operation.request_command_name(),
        canonical_executable: path_identity(executable),
        canonical_cwd: path_identity(cwd),
        argv,
        fixed_env,
        inherited_env_keys,
        source_fingerprints,
        safe_request,
        scope,
        session_id,
        profile_id,
        policy_ids,
    };
    let bytes = serde_json::to_vec(&payload).expect("projection digest payload serializes");
    let digest = Sha256::digest(&bytes);
    format!("sha256:{digest:x}")
}

#[derive(Serialize)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
enum PathIdentity {
    #[cfg(unix)]
    UnixBytes(String),
}

fn path_identity(path: &Path) -> PathIdentity {
    use std::os::unix::ffi::OsStrExt;
    let mut encoded = String::with_capacity(path.as_os_str().as_bytes().len() * 2);
    for byte in path.as_os_str().as_bytes() {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    PathIdentity::UnixBytes(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn write_exe(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    fn write_required_sources(pkg: &Path) {
        write_exe(&pkg.join("bin/ontarch"));
        write_exe(&pkg.join("bin/ontarch-bin-report"));
        write_exe(&pkg.join("bin/ontarch-bin-cleanup"));
        fs::create_dir_all(pkg.join("lib")).unwrap();
        fs::write(pkg.join("lib/common.sh"), b"#\n").unwrap();
        fs::write(pkg.join("lib/registry.sh"), b"#\n").unwrap();
        fs::create_dir_all(pkg.join("policies")).unwrap();
        fs::write(pkg.join("policies/takogami.agent.policy.toml"), b"#\n").unwrap();
        fs::write(pkg.join("policies/agent-bin.policy.toml"), b"#\n").unwrap();
        fs::create_dir_all(pkg.join("schemas")).unwrap();
        fs::write(pkg.join("schemas/bin-inventory.schema.json"), b"{}\n").unwrap();
        fs::write(pkg.join("schemas/bin-cleanup-plan.schema.json"), b"{}\n").unwrap();
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempdir().unwrap();
        let ws = temp.path().join("ws");
        let pkg = ws.join("packages/ontarch");
        let registry = pkg.join("registry");
        fs::create_dir_all(&registry).unwrap();
        write_required_sources(&pkg);
        (temp, registry, ws)
    }

    #[test]
    fn digest_stable_under_map_order() {
        let (_t, registry, ws) = fixture();
        let a = SealedProjectionPlan::seal(
            ProjectionOperation::BinReport,
            &registry,
            &ws,
            None,
            "sess-1".into(),
            "workspace-dev".into(),
            vec!["takogami.agent".into(), "agent-bin".into()],
        )
        .unwrap();
        let b = SealedProjectionPlan::seal(
            ProjectionOperation::BinReport,
            &registry,
            &ws,
            None,
            "sess-1".into(),
            "workspace-dev".into(),
            vec!["agent-bin".into(), "takogami.agent".into()],
        )
        .unwrap();
        assert_eq!(a.plan_digest(), b.plan_digest());
        assert!(a.plan_digest().starts_with("sha256:"));
        let path = a.fixed_env().get("PATH").unwrap();
        assert!(!path.contains("::"));
        for part in path.split(':') {
            assert!(Path::new(part).is_absolute());
            assert!(!part.is_empty());
        }
    }

    #[test]
    fn digest_changes_with_scope() {
        let (_t, registry, ws) = fixture();
        let none = SealedProjectionPlan::seal(
            ProjectionOperation::BinCleanupReportOnly,
            &registry,
            &ws,
            None,
            "sess-1".into(),
            "workspace-dev".into(),
            vec!["takogami.agent".into()],
        )
        .unwrap();
        let scoped = SealedProjectionPlan::seal(
            ProjectionOperation::BinCleanupReportOnly,
            &registry,
            &ws,
            Some(ValidatedBinScope::parse("Build/bin/wfos").unwrap()),
            "sess-1".into(),
            "workspace-dev".into(),
            vec!["takogami.agent".into()],
        )
        .unwrap();
        assert_ne!(none.plan_digest(), scoped.plan_digest());
    }

    #[test]
    fn rejects_missing_ontarch() {
        let temp = tempdir().unwrap();
        let ws = temp.path().join("ws");
        let registry = ws.join("packages/ontarch/registry");
        fs::create_dir_all(&registry).unwrap();
        let err = SealedProjectionPlan::seal(
            ProjectionOperation::BinReport,
            &registry,
            &ws,
            None,
            "s".into(),
            "p".into(),
            vec![],
        )
        .unwrap_err();
        assert_eq!(err.code(), "projection_tool_unavailable");
    }

    #[test]
    fn missing_required_projection_source_fails_seal() {
        let (_t, registry, ws) = fixture();
        let pkg = registry.parent().unwrap();
        fs::remove_file(pkg.join("schemas/bin-inventory.schema.json")).unwrap();
        let err = SealedProjectionPlan::seal(
            ProjectionOperation::BinReport,
            &registry,
            &ws,
            None,
            "s".into(),
            "p".into(),
            vec![],
        )
        .unwrap_err();
        assert_eq!(err.code(), "projection_contract_changed");
        assert!(err.message().contains("packages/ontarch/"));
        assert!(!err.message().contains(pkg.to_string_lossy().as_ref()));
    }

    #[test]
    fn symlink_projection_source_fails_seal() {
        let (_t, registry, ws) = fixture();
        let pkg = registry.parent().unwrap();
        let target = pkg.join("lib/common.sh");
        let link = pkg.join("lib/common.sh.link");
        fs::rename(&target, &link).unwrap();
        std::os::unix::fs::symlink(&link, &target).unwrap();
        let err = SealedProjectionPlan::seal(
            ProjectionOperation::BinReport,
            &registry,
            &ws,
            None,
            "s".into(),
            "p".into(),
            vec![],
        )
        .unwrap_err();
        assert_eq!(err.code(), "projection_contract_changed");
    }

    #[test]
    fn sealed_tool_path_contains_only_approved_absolute_directories() {
        let path = build_controller_tool_path().unwrap();
        crate::execution::validate_controller_path(&path).unwrap();
        for part in path.split(':') {
            assert!(Path::new(part).is_absolute());
            assert!(fs::metadata(part).unwrap().is_dir());
        }
    }
}

//! Sealed non-lifecycle projection plan and package-owned Ontarch resolution.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::op::ProjectionOperation;
use super::scope::ValidatedBinScope;
use crate::contracts::secure_file::sha256_hex_regular_nofollow;
use crate::contracts::types::RequestRecord;
use crate::contracts::{SourceFingerprint, fingerprint_regular_file_nofollow};

const DIGEST_PAYLOAD_VERSION: &str = "s7-projection-v1";

/// Ordered controller-owned helper search directories (never from caller PATH).
const CONTROLLER_SEARCH_DIRS: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/opt/homebrew/bin",
    "/usr/local/bin",
];

/// Required helpers for bin-report (and shared by cleanup).
const REPORT_HELPERS: &[&str] = &[
    "bash", "jq", "dirname", "basename", "readlink", "date", "du", "awk", "wc", "tr", "stat",
    "mkdir", "mktemp", "rm", "cat", "cp", "mv",
];

/// Extra helpers required only for cleanup operations.
const CLEANUP_EXTRA_HELPERS: &[&str] = &["grep", "sed", "head"];

/// Sealed helper identity: lookup path (PATH authority) + canonical target digest.
///
/// On platforms where helpers are symlinks (Alpine busybox applets), `lookup_path` is the
/// absolute path that provides the command name for child PATH, while `canonical_path` +
/// `digest` bind the resolved regular-file identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct HelperIdentity {
    pub name: String,
    #[serde(serialize_with = "serialize_path_hex")]
    pub lookup_path: PathBuf,
    #[serde(serialize_with = "serialize_path_hex")]
    pub canonical_path: PathBuf,
    pub algorithm: String,
    pub digest: String,
}

fn serialize_path_hex<S: serde::Serializer>(path: &Path, ser: S) -> Result<S::Ok, S::Error> {
    match path_identity(path) {
        PathIdentity::UnixBytes(hex) => ser.serialize_str(&hex),
    }
}

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
    helper_identities: Vec<HelperIdentity>,
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
        Self::seal_with_search_dirs(
            operation,
            registry_root,
            workspace_root,
            scope,
            session_id,
            profile_id,
            policy_ids,
            &controller_search_dirs(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn seal_with_search_dirs(
        operation: ProjectionOperation,
        registry_root: &Path,
        workspace_root: &Path,
        scope: Option<ValidatedBinScope>,
        session_id: String,
        profile_id: String,
        policy_ids: Vec<String>,
        search_dirs: &[PathBuf],
    ) -> Result<Self, ProjectionSealError> {
        let ontarch_pkg = registry_root
            .parent()
            .ok_or(ProjectionSealError::OntarchMissing)?;
        let executable = resolve_and_seal_ontarch(ontarch_pkg)?;
        let cwd = seal_cwd(workspace_root)?;

        let argv = operation.child_argv(scope.as_ref());

        let (helper_identities, path) = resolve_helper_authority(operation, search_dirs)?;
        crate::execution::validate_controller_path(&path).map_err(ProjectionSealError::ToolPath)?;

        let mut fixed_env = BTreeMap::new();
        fixed_env.insert("PANOPLY_AGENT".into(), "1".into());
        fixed_env.insert("NO_COLOR".into(), "1".into());
        fixed_env.insert(
            "WS_ROOT".into(),
            workspace_root.to_string_lossy().into_owned(),
        );
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
            &helper_identities,
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
            helper_identities,
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
    #[allow(dead_code)] // exercised by unit tests; retained for preflight diagnostics
    pub(crate) fn helper_identities(&self) -> &[HelperIdentity] {
        &self.helper_identities
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

    /// Re-validate sealed helper identities immediately before spawn.
    pub(crate) fn preflight_helpers(&self) -> Result<(), ProjectionSealError> {
        for helper in &self.helper_identities {
            // Lookup path must still resolve to the sealed canonical identity.
            let resolved = helper.lookup_path.canonicalize().map_err(|_| {
                ProjectionSealError::Identity(format!(
                    "helper {} lookup missing before spawn",
                    helper.name
                ))
            })?;
            if resolved != helper.canonical_path {
                return Err(ProjectionSealError::Identity(format!(
                    "helper {} lookup identity drifted",
                    helper.name
                )));
            }
            let fresh = seal_helper_at_path(&helper.name, &helper.canonical_path)?;
            if fresh.digest != helper.digest
                || fresh.algorithm != helper.algorithm
                || fresh.canonical_path != helper.canonical_path
            {
                return Err(ProjectionSealError::Identity(format!(
                    "helper identity drifted: {}",
                    helper.name
                )));
            }
        }
        let path = self
            .fixed_env
            .get("PATH")
            .ok_or_else(|| ProjectionSealError::ToolPath("sealed PATH missing".into()))?;
        crate::execution::validate_controller_path(path).map_err(ProjectionSealError::ToolPath)?;
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

fn controller_search_dirs() -> Vec<PathBuf> {
    #[cfg(test)]
    if let Some(dirs) = TEST_SEARCH_DIRS.with(|c| c.borrow().clone()) {
        return dirs;
    }
    CONTROLLER_SEARCH_DIRS.iter().map(PathBuf::from).collect()
}

#[cfg(test)]
thread_local! {
    static TEST_SEARCH_DIRS: std::cell::RefCell<Option<Vec<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

/// Resolve canonical non-symlink search directories from the controller candidate list.
pub(crate) fn resolve_controller_search_dirs(
    candidates: &[&str],
) -> Result<Vec<PathBuf>, ProjectionSealError> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for cand in candidates {
        let p = Path::new(cand);
        if !p.is_absolute() || cand.is_empty() {
            return Err(ProjectionSealError::ToolPath(
                "helper search directory must be absolute and non-empty".into(),
            ));
        }
        if cand.contains("/./")
            || cand.contains("/../")
            || *cand == "."
            || *cand == ".."
            || cand.ends_with("/.")
            || cand.ends_with("/..")
        {
            return Err(ProjectionSealError::ToolPath(
                "helper search directory must not contain relative components".into(),
            ));
        }
        let Ok(meta) = fs::symlink_metadata(p) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            // Reject symlink directory aliases — do not follow into the search set.
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        if is_world_writable(&meta) {
            continue;
        }
        let Ok(canonical) = p.canonicalize() else {
            continue;
        };
        // Deduplicate by canonical identity.
        if dirs.iter().any(|d| d == &canonical) {
            continue;
        }
        dirs.push(canonical);
    }
    if dirs.is_empty() {
        return Err(ProjectionSealError::ToolPath(
            "no approved absolute tool directories available".into(),
        ));
    }
    Ok(dirs)
}

fn required_helper_names(operation: ProjectionOperation) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = REPORT_HELPERS.to_vec();
    match operation {
        ProjectionOperation::BinCleanupReportOnly
        | ProjectionOperation::BinCleanupDryRun
        | ProjectionOperation::BinCleanupArchive
        | ProjectionOperation::BinCleanupDeleteApproved => {
            names.extend_from_slice(CLEANUP_EXTRA_HELPERS);
        }
        ProjectionOperation::BinReport => {}
    }
    names
}

fn resolve_helper_authority(
    operation: ProjectionOperation,
    search_candidates: &[PathBuf],
) -> Result<(Vec<HelperIdentity>, String), ProjectionSealError> {
    let cand_strs: Vec<&str> = search_candidates
        .iter()
        .filter_map(|p| p.to_str())
        .collect();
    let search_dirs = resolve_controller_search_dirs(&cand_strs)?;

    let mut helpers = Vec::new();
    for name in required_helper_names(operation) {
        helpers.push(resolve_named_helper(name, &search_dirs)?);
    }
    // Prefer fd, else find — seal the winner under its real name.
    let lookup = resolve_named_helper("fd", &search_dirs)
        .or_else(|_| resolve_named_helper("find", &search_dirs))
        .map_err(|_| {
            ProjectionSealError::ToolPath("required helper fd or find not found".into())
        })?;
    helpers.push(lookup);

    helpers.sort_by(|a, b| a.name.cmp(&b.name));
    let mut path_dirs: Vec<String> = Vec::new();
    for helper in &helpers {
        let parent = helper
            .lookup_path
            .parent()
            .ok_or_else(|| ProjectionSealError::ToolPath("helper missing parent directory".into()))?
            .to_path_buf();
        // PATH must expose the lookup directory (where the command name appears), not only
        // the canonical target parent (busybox lives in /bin while applets live in /usr/bin).
        let parent_str = parent.to_string_lossy().into_owned();
        if !path_dirs.iter().any(|d| d == &parent_str) {
            path_dirs.push(parent_str);
        }
    }
    let path = path_dirs.join(":");
    Ok((helpers, path))
}

fn resolve_named_helper(
    name: &str,
    search_dirs: &[PathBuf],
) -> Result<HelperIdentity, ProjectionSealError> {
    for dir in search_dirs {
        let candidate = dir.join(name);
        match try_seal_helper_candidate(name, &candidate) {
            Ok(id) => return Ok(id),
            Err(ProjectionSealError::ToolPath(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(ProjectionSealError::ToolPath(format!(
        "required helper {name} not found"
    )))
}

fn try_seal_helper_candidate(
    name: &str,
    candidate: &Path,
) -> Result<HelperIdentity, ProjectionSealError> {
    let _meta = fs::symlink_metadata(candidate)
        .map_err(|_| ProjectionSealError::ToolPath(format!("required helper {name} not found")))?;
    // Parent directory of the lookup path must not be a world-writable non-symlink dir.
    if let Some(parent) = candidate.parent() {
        let Ok(pm) = fs::symlink_metadata(parent) else {
            return Err(ProjectionSealError::ToolPath(format!(
                "helper {name} parent missing"
            )));
        };
        if pm.file_type().is_symlink() {
            return Err(ProjectionSealError::ToolPath(format!(
                "helper {name} parent must not be a symlink directory"
            )));
        }
        if is_world_writable(&pm) {
            return Err(ProjectionSealError::ToolPath(format!(
                "helper {name} parent directory is world-writable"
            )));
        }
    }
    // Allow symlink applets (Alpine busybox): seal the canonical regular-file target.
    let canonical = candidate
        .canonicalize()
        .map_err(|_| ProjectionSealError::ToolPath(format!("helper {name} does not resolve")))?;
    let target_meta = fs::symlink_metadata(&canonical)
        .map_err(|_| ProjectionSealError::ToolPath(format!("helper {name} target missing")))?;
    if target_meta.file_type().is_symlink() {
        return Err(ProjectionSealError::ToolPath(format!(
            "helper {name} resolved to a symlink"
        )));
    }
    if !target_meta.is_file() {
        return Err(ProjectionSealError::ToolPath(format!(
            "helper {name} is not a regular file"
        )));
    }
    if target_meta.permissions().mode() & 0o111 == 0 {
        return Err(ProjectionSealError::ToolPath(format!(
            "helper {name} is not executable"
        )));
    }
    if is_world_writable(&target_meta) {
        return Err(ProjectionSealError::ToolPath(format!(
            "helper {name} is world-writable"
        )));
    }
    let digest = sha256_hex_regular_nofollow(&canonical, name).map_err(|e| {
        ProjectionSealError::ToolPath(format!(
            "helper {name} fingerprint failed: {}",
            e.public_message()
        ))
    })?;
    if !candidate.is_absolute() {
        return Err(ProjectionSealError::ToolPath(format!(
            "helper {name} lookup path must be absolute"
        )));
    }
    Ok(HelperIdentity {
        name: name.to_string(),
        lookup_path: candidate.to_path_buf(),
        canonical_path: canonical,
        algorithm: "sha256".into(),
        digest,
    })
}

fn seal_helper_at_path(name: &str, path: &Path) -> Result<HelperIdentity, ProjectionSealError> {
    // Preflight path: `path` is the sealed canonical identity and must remain a regular file.
    let meta = fs::symlink_metadata(path).map_err(|_| {
        ProjectionSealError::Identity(format!("helper {name} missing before spawn"))
    })?;
    if meta.file_type().is_symlink() {
        return Err(ProjectionSealError::Identity(format!(
            "helper {name} must not be a symlink"
        )));
    }
    if !meta.is_file() {
        return Err(ProjectionSealError::Identity(format!(
            "helper {name} is not a regular file"
        )));
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(ProjectionSealError::Identity(format!(
            "helper {name} is not executable"
        )));
    }
    let canonical = path.canonicalize().map_err(|_| {
        ProjectionSealError::Identity(format!("helper {name} canonical identity drifted"))
    })?;
    if canonical != path {
        return Err(ProjectionSealError::Identity(format!(
            "helper {name} canonical identity drifted"
        )));
    }
    let digest = sha256_hex_regular_nofollow(&canonical, name).map_err(|e| {
        ProjectionSealError::Identity(format!(
            "helper {name} fingerprint failed: {}",
            e.public_message()
        ))
    })?;
    Ok(HelperIdentity {
        name: name.to_string(),
        lookup_path: path.to_path_buf(),
        canonical_path: canonical,
        algorithm: "sha256".into(),
        digest,
    })
}

fn is_world_writable(meta: &fs::Metadata) -> bool {
    meta.permissions().mode() & 0o002 != 0
}

/// Build a controller-owned PATH from sealed helper parent directories.
/// Does not read or inherit the caller's PATH.
#[allow(dead_code)] // unit-test and diagnostic entrypoint over resolve_helper_authority
pub(crate) fn build_controller_tool_path() -> Result<String, ProjectionSealError> {
    let (_helpers, path) =
        resolve_helper_authority(ProjectionOperation::BinReport, &controller_search_dirs())?;
    Ok(path)
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
    helper_identities: &[HelperIdentity],
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
        helper_identities: &'a [HelperIdentity],
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
        helper_identities,
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
    use std::os::unix::fs::{PermissionsExt, symlink};
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

    fn write_fake_helpers(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let mut names: Vec<&str> = REPORT_HELPERS.to_vec();
        names.extend_from_slice(CLEANUP_EXTRA_HELPERS);
        names.push("fd");
        names.push("find");
        for name in names {
            write_exe(&dir.join(name));
        }
    }

    fn with_search_dirs<R>(dirs: Vec<PathBuf>, f: impl FnOnce() -> R) -> R {
        TEST_SEARCH_DIRS.with(|c| {
            *c.borrow_mut() = Some(dirs);
            let out = f();
            *c.borrow_mut() = None;
            out
        })
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
        assert!(!a.helper_identities().is_empty());
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
        symlink(&link, &target).unwrap();
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
            let meta = fs::symlink_metadata(part).unwrap();
            assert!(!meta.file_type().is_symlink());
            assert!(meta.is_dir());
        }
    }

    #[test]
    fn controller_path_rejects_symlink_directory_alias() {
        let temp = tempdir().unwrap();
        let real = temp.path().join("realbin");
        let alias = temp.path().join("aliasbin");
        write_fake_helpers(&real);
        symlink(&real, &alias).unwrap();
        let err = resolve_controller_search_dirs(&[alias.to_str().unwrap()]).unwrap_err();
        assert_eq!(err.code(), "execution_contract");
    }

    #[test]
    fn controller_path_deduplicates_canonical_directory_identity() {
        let temp = tempdir().unwrap();
        let real = temp.path().join("bin");
        write_fake_helpers(&real);
        let canon = real.canonicalize().unwrap();
        // Same directory via identical absolute path twice.
        let dirs =
            resolve_controller_search_dirs(&[canon.to_str().unwrap(), canon.to_str().unwrap()])
                .unwrap();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0], canon);
    }

    #[test]
    fn controller_path_rejects_relative_and_empty_components() {
        assert!(resolve_controller_search_dirs(&[""]).is_err());
        assert!(resolve_controller_search_dirs(&["relative/bin"]).is_err());
        let err = resolve_controller_search_dirs(&["/usr/bin/../bin"]).unwrap_err();
        assert_eq!(err.code(), "execution_contract");
        let err = resolve_controller_search_dirs(&["/usr/bin/./extra"]).unwrap_err();
        assert_eq!(err.code(), "execution_contract");
    }

    #[test]
    fn helper_identity_change_after_seal_fails_preflight() {
        let temp = tempdir().unwrap();
        let tools = temp.path().join("tools");
        write_fake_helpers(&tools);
        let (_t, registry, ws) = fixture();
        let plan = with_search_dirs(vec![tools.clone()], || {
            SealedProjectionPlan::seal(
                ProjectionOperation::BinReport,
                &registry,
                &ws,
                None,
                "s".into(),
                "p".into(),
                vec![],
            )
            .unwrap()
        });
        let jq = plan
            .helper_identities()
            .iter()
            .find(|h| h.name == "jq")
            .unwrap();
        fs::write(&jq.canonical_path, b"#!/bin/sh\necho changed\n").unwrap();
        let mut perms = fs::metadata(&jq.canonical_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&jq.canonical_path, perms).unwrap();
        let err = plan.preflight_helpers().unwrap_err();
        assert_eq!(err.code(), "projection_contract_changed");
        assert!(err.message().contains("jq"));
        assert!(!err.message().contains(tools.to_str().unwrap()));
    }

    #[test]
    fn helper_removed_after_seal_fails_preflight() {
        let temp = tempdir().unwrap();
        let tools = temp.path().join("tools");
        write_fake_helpers(&tools);
        let (_t, registry, ws) = fixture();
        let plan = with_search_dirs(vec![tools.clone()], || {
            SealedProjectionPlan::seal(
                ProjectionOperation::BinReport,
                &registry,
                &ws,
                None,
                "s".into(),
                "p".into(),
                vec![],
            )
            .unwrap()
        });
        let jq = plan
            .helper_identities()
            .iter()
            .find(|h| h.name == "jq")
            .unwrap();
        fs::remove_file(&jq.canonical_path).unwrap();
        let err = plan.preflight_helpers().unwrap_err();
        assert_eq!(err.code(), "projection_contract_changed");
    }

    #[test]
    fn helper_symlink_replacement_after_seal_fails_preflight() {
        let temp = tempdir().unwrap();
        let tools = temp.path().join("tools");
        write_fake_helpers(&tools);
        let (_t, registry, ws) = fixture();
        let plan = with_search_dirs(vec![tools.clone()], || {
            SealedProjectionPlan::seal(
                ProjectionOperation::BinReport,
                &registry,
                &ws,
                None,
                "s".into(),
                "p".into(),
                vec![],
            )
            .unwrap()
        });
        let jq = plan
            .helper_identities()
            .iter()
            .find(|h| h.name == "jq")
            .unwrap();
        let real = tools.join("jq.real");
        // Replace the lookup entry with a symlink so re-resolve drifts from the sealed
        // canonical regular-file identity.
        fs::rename(&jq.lookup_path, &real).unwrap();
        symlink(&real, &jq.lookup_path).unwrap();
        let err = plan.preflight_helpers().unwrap_err();
        assert_eq!(err.code(), "projection_contract_changed");
    }
}

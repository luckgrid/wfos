//! Sealed pre-policy plan and S5 handoff seam.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::explain::ExecutableProvenance;
use crate::contracts::{DiagnosticRecord, ResolvedCommand};

const DIGEST_PAYLOAD_VERSION: &str = "s4.1-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Agent,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyRequestView {
    pub unit_id: String,
    pub verb: String,
    pub profile_id: String,
}

/// Canonical Takogami operation for request-layer policy matching.
#[derive(Debug, Clone, Serialize)]
pub struct RequestedOperation {
    pub program: String,
    pub argv: Vec<String>,
    pub unit_id: String,
    pub verb: String,
    pub explain_requested: bool,
    pub execute_requested: bool,
}

impl RequestedOperation {
    pub fn from_resolution(unit_id: &str, verb: &str, explain: bool, execute: bool) -> Self {
        let mut argv = vec![verb.to_string(), unit_id.to_string()];
        if explain {
            argv.push("--explain".into());
        }
        if execute {
            argv.push("--execute".into());
        }
        Self {
            program: "takogami".into(),
            argv,
            unit_id: unit_id.into(),
            verb: verb.into(),
            explain_requested: explain,
            execute_requested: execute,
        }
    }
}

/// Sealed plan: constructible only via [`SealedExecutionPlan::seal`].
#[derive(Debug, Clone)]
pub struct SealedExecutionPlan {
    resolved: ResolvedCommand,
    executable_path: PathBuf,
    cwd_path: PathBuf,
    source_manifest_paths: Vec<PathBuf>,
    executable_provenance: ExecutableProvenance,
    diagnostics: Vec<DiagnosticRecord>,
    plan_digest: String,
}

impl SealedExecutionPlan {
    pub(crate) fn seal(
        mut resolved: ResolvedCommand,
        executable_path: PathBuf,
        cwd_path: PathBuf,
        mut source_manifest_paths: Vec<PathBuf>,
        executable_provenance: ExecutableProvenance,
        mut diagnostics: Vec<DiagnosticRecord>,
    ) -> Self {
        resolved.native_manifests.sort();
        resolved.native_manifests.dedup();
        resolved.env_keys.sort();
        resolved.env_keys.dedup();
        resolved.policy_ids.sort();
        resolved.policy_ids.dedup();
        resolved
            .registry_generation
            .source_fingerprints
            .sort_by(|a, b| {
                a.path
                    .cmp(&b.path)
                    .then(a.algorithm.cmp(&b.algorithm))
                    .then(a.digest.cmp(&b.digest))
            });
        resolved.registry_generation.source_fingerprints.dedup();
        source_manifest_paths.sort();
        source_manifest_paths.dedup();
        diagnostics.sort_by(|a, b| a.code.cmp(&b.code).then(a.message.cmp(&b.message)));
        diagnostics.dedup();
        let plan_digest = compute_plan_digest(
            &resolved,
            &executable_path,
            &cwd_path,
            &source_manifest_paths,
            &executable_provenance,
            &diagnostics,
        );
        Self {
            resolved,
            executable_path,
            cwd_path,
            source_manifest_paths,
            executable_provenance,
            diagnostics,
            plan_digest,
        }
    }

    pub fn resolved(&self) -> &ResolvedCommand {
        &self.resolved
    }

    pub fn executable_path(&self) -> &PathBuf {
        &self.executable_path
    }

    pub fn cwd_path(&self) -> &PathBuf {
        &self.cwd_path
    }

    pub fn source_manifest_paths(&self) -> &[PathBuf] {
        &self.source_manifest_paths
    }

    pub fn executable_provenance(&self) -> &ExecutableProvenance {
        &self.executable_provenance
    }

    pub fn diagnostics(&self) -> &[DiagnosticRecord] {
        &self.diagnostics
    }

    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
}

fn compute_plan_digest(
    resolved: &ResolvedCommand,
    executable_path: &Path,
    cwd_path: &Path,
    source_manifest_paths: &[PathBuf],
    executable_provenance: &ExecutableProvenance,
    diagnostics: &[DiagnosticRecord],
) -> String {
    #[derive(Serialize)]
    struct DigestPayload<'a> {
        version: &'static str,
        resolved: &'a ResolvedCommand,
        canonical_executable: PathIdentity,
        canonical_cwd: PathIdentity,
        canonical_source_manifests: Vec<PathIdentity>,
        executable_provenance: &'a ExecutableProvenance,
        diagnostics: &'a [DiagnosticRecord],
    }
    let payload = DigestPayload {
        version: DIGEST_PAYLOAD_VERSION,
        resolved,
        canonical_executable: path_identity(executable_path),
        canonical_cwd: path_identity(cwd_path),
        canonical_source_manifests: source_manifest_paths
            .iter()
            .map(|path| path_identity(path))
            .collect(),
        executable_provenance,
        diagnostics,
    };
    // Every path uses a reversible platform-native encoding; display rendering is never hashed.
    let bytes = serde_json::to_vec(&payload).expect("plan digest payload serializes");
    let digest = Sha256::digest(&bytes);
    format!("sha256:{digest:x}")
}

#[derive(Serialize)]
#[serde(tag = "encoding", content = "value", rename_all = "snake_case")]
enum PathIdentity {
    #[cfg(unix)]
    UnixBytes(String),
    #[cfg(windows)]
    WindowsWide(Vec<u16>),
}

fn path_identity(path: &Path) -> PathIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut encoded = String::with_capacity(path.as_os_str().as_bytes().len() * 2);
        for byte in path.as_os_str().as_bytes() {
            use std::fmt::Write;
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        PathIdentity::UnixBytes(encoded)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        PathIdentity::WindowsWide(path.as_os_str().encode_wide().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ExecutionClass, RegistryGeneration, SCHEMA_VERSION};
    use crate::resolution::{ExecutableProvenance, ExecutableSelectionSource};

    fn provenance() -> ExecutableProvenance {
        ExecutableProvenance {
            selection_source: ExecutableSelectionSource::Path,
            tool_id: None,
            path_index: Some(0),
            display_path: None,
        }
    }

    fn sample_resolved(session_id: &str) -> ResolvedCommand {
        ResolvedCommand {
            schema_version: SCHEMA_VERSION.into(),
            session_id: session_id.into(),
            unit_id: "demo".into(),
            verb: "build".into(),
            descriptor_path: "registry/sources/descriptors/demo.descriptor.toml".into(),
            descriptor_fingerprint: "sha256:abc".into(),
            native_manifests: vec!["moon.yml".into()],
            backend: "moon".into(),
            adapter: "moon-task".into(),
            program: "moon".into(),
            argv: vec!["run".into(), "demo:build".into()],
            cwd: "demo".into(),
            env_keys: vec!["PATH".into()],
            profile_id: "workspace-dev".into(),
            policy_ids: vec!["panoply.agent".into()],
            registry_generation: RegistryGeneration {
                generated_at: "2026-07-21T00:00:00Z".into(),
                source_fingerprints: vec![],
            },
            execution_class: ExecutionClass::Direct,
            runtime_provider: None,
        }
    }

    #[test]
    fn seal_twice_same_session_yields_equal_digest_and_resolved() {
        let resolved = sample_resolved("tkg_fixed");
        let a = SealedExecutionPlan::seal(
            resolved.clone(),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![PathBuf::from("/ws/demo/moon.yml")],
            provenance(),
            vec![],
        );
        let b = SealedExecutionPlan::seal(
            resolved.clone(),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![PathBuf::from("/ws/demo/moon.yml")],
            provenance(),
            vec![],
        );
        assert_eq!(a.plan_digest(), b.plan_digest());
        assert!(a.plan_digest().starts_with("sha256:"));
        assert_eq!(a.resolved(), b.resolved());
        assert_eq!(
            serde_json::to_vec(a.resolved()).unwrap(),
            serde_json::to_vec(b.resolved()).unwrap()
        );
    }

    #[test]
    fn digest_includes_session_id() {
        let a = SealedExecutionPlan::seal(
            sample_resolved("tkg_a"),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );
        let b = SealedExecutionPlan::seal(
            sample_resolved("tkg_b"),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );
        assert_ne!(a.plan_digest(), b.plan_digest());
    }

    #[test]
    fn digest_binds_private_execution_paths() {
        let resolved = sample_resolved("tkg_fixed");
        let base = SealedExecutionPlan::seal(
            resolved.clone(),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![PathBuf::from("/ws/demo/moon.yml")],
            provenance(),
            vec![],
        );
        let executable_changed = SealedExecutionPlan::seal(
            resolved.clone(),
            PathBuf::from("/other/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![PathBuf::from("/ws/demo/moon.yml")],
            provenance(),
            vec![],
        );
        let cwd_changed = SealedExecutionPlan::seal(
            resolved.clone(),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/other"),
            vec![PathBuf::from("/ws/demo/moon.yml")],
            provenance(),
            vec![],
        );
        let manifest_changed = SealedExecutionPlan::seal(
            resolved,
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![PathBuf::from("/ws/demo/Cargo.toml")],
            provenance(),
            vec![],
        );

        assert_ne!(base.plan_digest(), executable_changed.plan_digest());
        assert_ne!(base.plan_digest(), cwd_changed.plan_digest());
        assert_ne!(base.plan_digest(), manifest_changed.plan_digest());
    }

    #[test]
    fn digest_normalizes_sets_but_binds_argv_policy_and_provider() {
        let mut base_resolved = sample_resolved("tkg_fixed");
        base_resolved.policy_ids = vec!["z-policy".into(), "a-policy".into()];
        let base = SealedExecutionPlan::seal(
            base_resolved.clone(),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );

        let mut reordered = base_resolved.clone();
        reordered.policy_ids.reverse();
        let reordered = SealedExecutionPlan::seal(
            reordered,
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );
        assert_eq!(base.plan_digest(), reordered.plan_digest());

        let mut argv_changed = base_resolved.clone();
        argv_changed.argv = vec!["run demo:build".into()];
        let argv_changed = SealedExecutionPlan::seal(
            argv_changed,
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );
        assert_ne!(base.plan_digest(), argv_changed.plan_digest());

        let mut policies_changed = base_resolved.clone();
        policies_changed.policy_ids.push("another-policy".into());
        let policies_changed = SealedExecutionPlan::seal(
            policies_changed,
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );
        assert_ne!(base.plan_digest(), policies_changed.plan_digest());

        let mut provider_changed = base_resolved;
        provider_changed.execution_class = ExecutionClass::InteractiveSession;
        provider_changed.runtime_provider = Some("herdr".into());
        let provider_changed = SealedExecutionPlan::seal(
            provider_changed,
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            provenance(),
            vec![],
        );
        assert_ne!(base.plan_digest(), provider_changed.plan_digest());
    }
}

//! Sealed pre-policy plan and S5 handoff seam.

use std::path::PathBuf;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::contracts::{DiagnosticRecord, ResolvedCommand};
use crate::registry::{PolicyRecord, ProfileRecord};

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

/// Sealed plan: constructible only via [`SealedExecutionPlan::seal`].
#[derive(Debug, Clone)]
pub struct SealedExecutionPlan {
    resolved: ResolvedCommand,
    executable_path: PathBuf,
    cwd_path: PathBuf,
    source_manifest_paths: Vec<PathBuf>,
    diagnostics: Vec<DiagnosticRecord>,
    plan_digest: String,
}

impl SealedExecutionPlan {
    pub(crate) fn seal(
        resolved: ResolvedCommand,
        executable_path: PathBuf,
        cwd_path: PathBuf,
        source_manifest_paths: Vec<PathBuf>,
        diagnostics: Vec<DiagnosticRecord>,
    ) -> Self {
        let plan_digest = compute_plan_digest(&resolved);
        Self {
            resolved,
            executable_path,
            cwd_path,
            source_manifest_paths,
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

    pub fn diagnostics(&self) -> &[DiagnosticRecord] {
        &self.diagnostics
    }

    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
}

/// S5 input seam — not evaluated in S4.
#[derive(Debug, Clone)]
pub struct PolicyEvaluationInput {
    pub actor: Actor,
    pub request: PolicyRequestView,
    pub plan: SealedExecutionPlan,
    pub profile: ProfileRecord,
    pub policies: Vec<PolicyRecord>,
}

fn compute_plan_digest(resolved: &ResolvedCommand) -> String {
    #[derive(Serialize)]
    struct DigestPayload<'a> {
        session_id: &'a str,
        unit_id: &'a str,
        verb: &'a str,
        descriptor_path: &'a str,
        descriptor_fingerprint: &'a str,
        native_manifests: &'a [String],
        backend: &'a str,
        adapter: &'a str,
        program: &'a str,
        argv: &'a [String],
        cwd: &'a str,
        env_keys: &'a [String],
        profile_id: &'a str,
        policy_ids: &'a [String],
        execution_class: &'a str,
        runtime_provider: Option<&'a str>,
    }
    let payload = DigestPayload {
        session_id: &resolved.session_id,
        unit_id: &resolved.unit_id,
        verb: &resolved.verb,
        descriptor_path: &resolved.descriptor_path,
        descriptor_fingerprint: &resolved.descriptor_fingerprint,
        native_manifests: &resolved.native_manifests,
        backend: &resolved.backend,
        adapter: &resolved.adapter,
        program: &resolved.program,
        argv: &resolved.argv,
        cwd: &resolved.cwd,
        env_keys: &resolved.env_keys,
        profile_id: &resolved.profile_id,
        policy_ids: &resolved.policy_ids,
        execution_class: resolved.execution_class.as_str(),
        runtime_provider: resolved.runtime_provider.as_deref(),
    };
    // DigestPayload is plain &str/slices — serialization only fails if the allocator does.
    let bytes = serde_json::to_vec(&payload).expect("plan digest payload serializes");
    let digest = Sha256::digest(&bytes);
    format!("sha256:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ExecutionClass, RegistryGeneration, SCHEMA_VERSION};

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
            vec![],
        );
        let b = SealedExecutionPlan::seal(
            resolved.clone(),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![PathBuf::from("/ws/demo/moon.yml")],
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
            vec![],
        );
        let b = SealedExecutionPlan::seal(
            sample_resolved("tkg_b"),
            PathBuf::from("/ws/bin/moon"),
            PathBuf::from("/ws/demo"),
            vec![],
            vec![],
        );
        assert_ne!(a.plan_digest(), b.plan_digest());
    }
}

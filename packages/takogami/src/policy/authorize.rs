//! Executor seam for evaluator-authorized plans (no production spawn in S5).

use super::evaluate::AuthorizedExecutionPlan;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorResult {
    Unavailable,
    /// Test-only sentinel.
    SpyReached,
}

pub trait Executor {
    fn execute(&self, plan: &AuthorizedExecutionPlan) -> ExecutorResult;
}

/// Production S5 executor — never starts a child.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableExecutor;

impl Executor for UnavailableExecutor {
    fn execute(&self, _plan: &AuthorizedExecutionPlan) -> ExecutorResult {
        ExecutorResult::Unavailable
    }
}

/// Test spy that counts how many times it was reached.
#[derive(Debug, Default)]
pub struct SpyExecutor {
    pub calls: std::cell::Cell<u32>,
}

impl Executor for SpyExecutor {
    fn execute(&self, _plan: &AuthorizedExecutionPlan) -> ExecutorResult {
        self.calls.set(self.calls.get() + 1);
        ExecutorResult::SpyReached
    }
}

impl SpyExecutor {
    /// Convenience: whether the spy has been invoked at least once.
    pub fn reached(&self) -> bool {
        self.calls.get() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::fingerprint_file;
    use crate::policy::{PolicyEvaluationResult, evaluate_policy};
    use crate::registry::{RegistryAccess, RegistryPaths};
    use crate::resolution::{FixedIdGenerator, LifecycleVerb, ResolutionRequest, resolve};
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    struct DemoHandoff {
        _temp: TempDir,
        input: crate::resolution::PolicyEvaluationInput,
    }

    #[test]
    fn spy_records_reachability() {
        // AuthorizedExecutionPlan construction and its proof are private to evaluate.rs.
        // Gate/Deny cannot be wrapped into an AuthorizedExecutionPlan from the public API.
        let spy = SpyExecutor::default();
        assert_eq!(spy.calls.get(), 0);
        assert!(!spy.reached());
    }

    #[test]
    fn spy_execute_increments_call_count() {
        let handoff = resolve_demo_handoff();
        let result = evaluate_policy(&handoff.input);
        let PolicyEvaluationResult::Authorized(plan) = result else {
            panic!("expected dual-Allow; got {result:?}");
        };
        let decision = plan.policy_decision();
        let explanation = plan.policy_explanation();
        assert!(
            matches!(decision, crate::contracts::PolicyDecision::Allow { .. }),
            "decision={decision:?} child={}",
            explanation.child.decision
        );
        let spy = SpyExecutor::default();
        assert_eq!(spy.calls.get(), 0);
        assert_eq!(spy.execute(&plan), ExecutorResult::SpyReached);
        assert_eq!(spy.calls.get(), 1);
        assert!(spy.reached());
        let _ = spy.execute(&plan);
        assert_eq!(spy.calls.get(), 2);
    }

    fn resolve_demo_handoff() -> DemoHandoff {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        fs::create_dir_all(&workspace).unwrap();
        copy_tree(&fixture, &workspace);

        let path_dir = workspace.join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["moon", "demo-bin", "rg"] {
            let p = path_dir.join(name);
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        let registry = workspace.join("registry");
        write_demo_units(&workspace, &registry);

        let access = RegistryAccess::new(RegistryPaths {
            registry_root: registry,
            workspace_root: workspace,
        });
        let request = ResolutionRequest {
            session_id: "tkg_auth_spy".into(),
            unit_id: "demo".into(),
            verb: LifecycleVerb::Build,
            explicit_profile: None,
            explain: false,
            execute_requested: false,
        };
        let mut id_gen = FixedIdGenerator {
            id: "tkg_unused".into(),
        };
        let success =
            resolve(&access, request, vec![path_dir], None, &mut id_gen).expect("resolve demo");
        DemoHandoff {
            input: success.policy_evaluation_input(),
            _temp: temp,
        }
    }

    fn write_demo_units(workspace: &Path, registry: &Path) {
        let desc_dir = registry.join("sources/descriptors");
        let mut fps = Vec::new();
        let mut units = Vec::new();
        for entry in fs::read_dir(&desc_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let authored: toml::Value =
                toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let id = authored["id"].as_str().unwrap().to_string();
            if id != "demo" {
                continue;
            }
            let rel = format!(
                "registry/sources/descriptors/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            fps.push(fingerprint_file(&workspace.join(&rel), &rel).unwrap());
            let entrypoints: serde_json::Value =
                serde_json::to_value(authored.get("entrypoints").unwrap()).unwrap();
            let native: serde_json::Value = serde_json::to_value(
                authored
                    .get("native")
                    .and_then(|n| n.get("manifests"))
                    .unwrap(),
            )
            .unwrap();
            units.push(serde_json::json!({
                "id": id,
                "kind": "package",
                "path": "demo",
                "native_manifests": native,
                "entrypoints": entrypoints,
                "source": "central",
                "provides": [],
                "requires": [],
            }));
        }
        let doc = serde_json::json!({
            "generated_at": "2026-07-21T00:00:00Z",
            "registry_generation": {
                "generated_at": "2026-07-21T00:00:00Z",
                "source_fingerprints": fps,
            },
            "summary": {"total": units.len()},
            "units": units,
        });
        fs::write(
            registry.join("units.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
    }

    fn copy_tree(src: &Path, dst: &Path) {
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir_all(&to).unwrap();
                copy_tree(&entry.path(), &to);
            } else {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::copy(entry.path(), &to).unwrap();
            }
        }
    }
}

//! Profile/policy enforcement. No process spawn; no RuntimeCommandRecord.

// ponytail: assert at compile/test time that this tree stays spawn-free.
#[cfg(test)]
mod no_process_api {
    #[test]
    fn policy_sources_omit_process_apis() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/policy");
        let mut hits = Vec::new();
        for entry in std::fs::read_dir(&root).expect("policy dir") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("mod.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for needle in ["std::process::Command", "tokio::process", "Command::new("] {
                if text.contains(needle) {
                    hits.push(format!("{}: {needle}", path.display()));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "policy/ must not import process APIs: {hits:?}"
        );
    }
}

mod authorize;
mod classify;
mod command;
mod evaluate;
mod explain;
mod normalize;
mod paths;
mod raw;

pub use authorize::{Executor, ExecutorResult, SpyExecutor, UnavailableExecutor};
pub use evaluate::{
    AuthorizedExecutionPlan, PolicyContractError, PolicyEvaluationExplanation,
    PolicyEvaluationResult, PolicyLayer, PolicyLayerResult, RejectedPolicyOutcome, evaluate_policy,
};
pub use explain::{render_human_policy_section, render_human_policy_summary};
pub use normalize::{CanonicalRule, Effect, MatcherKind, NormalizedPolicySet, OriginKind};
pub use raw::{PolicyEnforcementRecord, ProfileEnforcement, RemoteWritePolicy};

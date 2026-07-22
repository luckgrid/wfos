//! Pure lifecycle resolution (E09.S4). No process spawn; no RuntimeCommandRecord.

// ponytail: assert at compile/test time that this tree stays spawn-free.
#[cfg(test)]
mod no_process_api {
    #[test]
    fn resolution_sources_omit_process_apis() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/resolution");
        let mut hits = Vec::new();
        for entry in std::fs::read_dir(&root).expect("resolution dir") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            // Skip this file: it documents the forbidden needles.
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
            "resolution/ must not import process APIs: {hits:?}"
        );
    }
}

mod entrypoint;
mod executable;
mod explain;
mod paths;
mod plan;
mod profile;
mod request;
mod resolver;

pub use explain::{
    CommandExplanation, ExecutableProvenance, ExecutableSelectionSource, ExecutionExplanation,
    FreshnessExplanation, IsolationExplanation, PartialRequestView, PartialResolutionTrace,
    PartialUnitView, PolicyReferenceExplanation, ProfileExplanation, ResolutionExplanation,
    ResolutionStep, SafeEntrypointView, SafeSourceView, SourceExplanation, UnitExplanation,
    render_human_explanation, render_human_partial_explanation, render_human_summary,
};
pub use plan::{Actor, PolicyEvaluationInput, PolicyRequestView, SealedExecutionPlan};
pub use request::{
    CorrelationIdGenerator, DefaultIdGenerator, FixedIdGenerator, LifecycleVerb, ResolutionRequest,
};
pub use resolver::{
    BackendKind, ResolutionCode, ResolveSuccess, Resolver, ResolverInputs, resolve,
};

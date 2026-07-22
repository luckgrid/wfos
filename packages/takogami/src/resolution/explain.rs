//! Typed explanation model and human renderer.

use serde::Serialize;

use super::plan::SealedExecutionPlan;
use super::profile::SelectedProfile;
use crate::contracts::DiagnosticRecord;
use crate::registry::Freshness;

#[derive(Debug, Clone, Serialize)]
pub struct ResolutionExplanation {
    pub session_id: String,
    pub mode: String,
    pub unit: UnitExplanation,
    pub sources: SourceExplanation,
    pub command: CommandExplanation,
    pub execution: ExecutionExplanation,
    pub profile: ProfileExplanation,
    pub policies: Vec<PolicyReferenceExplanation>,
    pub freshness: FreshnessExplanation,
    pub isolation: IsolationExplanation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_digest: Option<String>,
    pub completed_steps: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitExplanation {
    pub id: String,
    pub verb: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceExplanation {
    pub descriptor: String,
    pub descriptor_fingerprint: String,
    pub native_manifests: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandExplanation {
    pub backend: String,
    pub adapter: String,
    pub program: String,
    pub arguments: Vec<String>,
    pub cwd: String,
    pub env_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionExplanation {
    pub execution_class: String,
    pub runtime_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileExplanation {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyReferenceExplanation {
    pub id: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreshnessExplanation {
    pub registry_cache: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IsolationExplanation {
    pub mode: Option<String>,
    pub jj: Option<String>,
    pub enforced: bool,
}

pub fn explanation_from_plan(
    plan: &SealedExecutionPlan,
    selected: &SelectedProfile,
    freshness: Freshness,
) -> ResolutionExplanation {
    let r = plan.resolved();
    ResolutionExplanation {
        session_id: r.session_id.clone(),
        mode: "plan_only".into(),
        unit: UnitExplanation {
            id: r.unit_id.clone(),
            verb: r.verb.clone(),
        },
        sources: SourceExplanation {
            descriptor: r.descriptor_path.clone(),
            descriptor_fingerprint: r.descriptor_fingerprint.clone(),
            native_manifests: r.native_manifests.clone(),
        },
        command: CommandExplanation {
            backend: r.backend.clone(),
            adapter: r.adapter.clone(),
            program: r.program.clone(),
            arguments: r.argv.clone(),
            cwd: r.cwd.clone(),
            env_keys: r.env_keys.clone(),
        },
        execution: ExecutionExplanation {
            execution_class: r.execution_class.as_str().into(),
            runtime_provider: r.runtime_provider.clone(),
        },
        profile: ProfileExplanation {
            id: selected.profile.id.clone(),
        },
        policies: selected
            .policy_origins
            .iter()
            .map(|(id, origin)| PolicyReferenceExplanation {
                id: id.clone(),
                origin: origin.clone(),
            })
            .collect(),
        freshness: FreshnessExplanation {
            registry_cache: freshness.as_str().into(),
        },
        isolation: IsolationExplanation {
            mode: selected.profile.isolation_mode.clone(),
            jj: selected.profile.isolation_jj.clone(),
            enforced: false,
        },
        plan_digest: Some(plan.plan_digest().to_string()),
        completed_steps: vec![
            "correlation_id".into(),
            "registry".into(),
            "unit".into(),
            "descriptor".into(),
            "entrypoint".into(),
            "cwd".into(),
            "manifests".into(),
            "executable".into(),
            "profile".into(),
            "plan".into(),
        ],
        diagnostics: plan.diagnostics().to_vec(),
    }
}

/// Human `--explain` field order per plan §9.1.
pub fn render_human_explanation(ex: &ResolutionExplanation) -> String {
    let mut lines = Vec::new();
    lines.push("Plan only — no process started".into());
    lines.push(format!("Session: {}", ex.session_id));
    lines.push(format!("Unit: {}", ex.unit.id));
    lines.push(format!("Verb: {}", ex.unit.verb));
    lines.push(format!("Descriptor: {}", ex.sources.descriptor));
    lines.push(format!(
        "Descriptor fingerprint: {}",
        ex.sources.descriptor_fingerprint
    ));
    lines.push(format!(
        "Registry freshness: {}",
        ex.freshness.registry_cache
    ));
    lines.push("Native manifests:".into());
    if ex.sources.native_manifests.is_empty() {
        lines.push("  (none)".into());
    } else {
        for m in &ex.sources.native_manifests {
            lines.push(format!("  - {m}"));
        }
    }
    lines.push(format!("Backend: {}", ex.command.backend));
    lines.push(format!("Adapter: {}", ex.command.adapter));
    lines.push(format!("Execution class: {}", ex.execution.execution_class));
    lines.push(format!(
        "Runtime provider: {}",
        ex.execution.runtime_provider.as_deref().unwrap_or("none")
    ));
    lines.push(format!("Program: {}", ex.command.program));
    lines.push("Arguments:".into());
    for (i, a) in ex.command.arguments.iter().enumerate() {
        lines.push(format!("  [{i}] {a}"));
    }
    lines.push(format!("Working directory: {}", ex.command.cwd));
    lines.push("Environment keys:".into());
    if ex.command.env_keys.is_empty() {
        lines.push("  (none)".into());
    } else {
        for k in &ex.command.env_keys {
            lines.push(format!("  - {k}"));
        }
    }
    lines.push(format!("Profile: {}", ex.profile.id));
    lines.push("Policies:".into());
    if ex.policies.is_empty() {
        lines.push("  (none)".into());
    } else {
        for p in &ex.policies {
            lines.push(format!("  - {} ({})", p.id, p.origin));
        }
    }
    lines.push(format!(
        "Declared isolation: {}; jj={}; enforced={}",
        ex.isolation.mode.as_deref().unwrap_or("none"),
        ex.isolation.jj.as_deref().unwrap_or("none"),
        ex.isolation.enforced
    ));
    if let Some(d) = &ex.plan_digest {
        lines.push(format!("Plan digest: {d}"));
    }
    if ex.diagnostics.is_empty() {
        lines.push("Diagnostics: none".into());
    } else {
        lines.push("Diagnostics:".into());
        for d in &ex.diagnostics {
            lines.push(format!("  - {}: {}", d.code, d.message));
        }
    }
    lines.join("\n")
}

pub fn render_human_summary(ex: &ResolutionExplanation) -> String {
    format!(
        "Plan only — {} {} → {} {:?}\nSession: {}\nPlan digest: {}\nProfile: {}\nFreshness: {}",
        ex.unit.verb,
        ex.unit.id,
        ex.command.program,
        ex.command.arguments,
        ex.session_id,
        ex.plan_digest.as_deref().unwrap_or("(none)"),
        ex.profile.id,
        ex.freshness.registry_cache,
    )
}

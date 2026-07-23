mod envelope;

pub use crate::contracts::{CommandEnvelope, DiagnosticRecord, EnvelopeMetrics};
pub use envelope::emit_json;

use crate::doctor::DoctorReport;
use crate::error::ControllerError;
use crate::policy::{
    AuthorizedExecutionPlan, PolicyEvaluationExplanation, RejectedPolicyOutcome,
    render_human_policy_section, render_human_policy_summary,
};
use crate::registry::Freshness;
use crate::resolution::{
    PartialResolutionTrace, ResolutionExplanation, SealedExecutionPlan, render_human_explanation,
    render_human_partial_explanation, render_human_summary,
};
use serde::Serialize;
use std::io::{self, Write};
use std::path::Path;

#[derive(Serialize)]
struct PolicyFailurePlanSummary<'a> {
    schema_version: &'a str,
    session_id: &'a str,
    unit_id: &'a str,
    verb: &'a str,
    backend: &'a str,
    adapter: &'a str,
    program_basename: &'a str,
    profile_id: &'a str,
    policy_ids: &'a [String],
    execution_class: &'a str,
    runtime_provider: Option<&'a str>,
    plan_digest: &'a str,
}

impl<'a> PolicyFailurePlanSummary<'a> {
    fn from_plan(plan: &'a SealedExecutionPlan) -> Self {
        let resolved = plan.resolved();
        let program_basename = Path::new(&resolved.program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unavailable");
        Self {
            schema_version: &resolved.schema_version,
            session_id: &resolved.session_id,
            unit_id: &resolved.unit_id,
            verb: &resolved.verb,
            backend: &resolved.backend,
            adapter: &resolved.adapter,
            program_basename,
            profile_id: &resolved.profile_id,
            policy_ids: &resolved.policy_ids,
            execution_class: resolved.execution_class.as_str(),
            runtime_provider: resolved.runtime_provider.as_deref(),
            plan_digest: plan.plan_digest(),
        }
    }
}

/// Write command output to stdout; diagnostics go to stderr when not JSON.
pub struct OutputSink {
    pub json: bool,
    pub no_color: bool,
}

impl OutputSink {
    pub fn emit_envelope(&self, envelope: &CommandEnvelope) -> io::Result<()> {
        emit_json(envelope)
    }

    pub fn emit_success(
        &self,
        command: &str,
        data: serde_json::Value,
        freshness: Option<Freshness>,
        human_lines: &[String],
    ) -> io::Result<u8> {
        if self.json {
            let mut envelope = CommandEnvelope::ok(command, Some(data));
            if let Some(f) = freshness {
                envelope.metrics = Some(EnvelopeMetrics {
                    registry_cache: f.as_str().to_string(),
                    output_bytes: 0,
                    compressor: "none".into(),
                    gain: None,
                });
            }
            emit_json(&envelope)?;
        } else {
            for line in human_lines {
                writeln!(io::stdout(), "{line}")?;
            }
        }
        Ok(crate::exit_codes::SUCCESS)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_plan_with_policy(
        &self,
        command: &str,
        authorized: &AuthorizedExecutionPlan,
        explanation: &ResolutionExplanation,
        freshness: Freshness,
    ) -> io::Result<u8> {
        let plan = authorized.plan();
        let policy_decision = authorized.policy_decision();
        let policy_explanation = authorized.policy_explanation();
        let execution_requested = authorized.request().execute_requested;
        let execution_authorized = true;
        if self.json {
            let data = serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
                "policy_decision": policy_decision,
                "policy": {
                    "request": policy_explanation.request,
                    "child": policy_explanation.child,
                    "execution_authorized": execution_authorized,
                    "approval_transport": policy_explanation.approval_transport,
                },
                "execution_authorized": execution_authorized,
                "execution_requested": execution_requested,
            });
            let mut envelope = CommandEnvelope::ok(command, Some(data));
            envelope.session_id = Some(plan.resolved().session_id.clone());
            envelope.diagnostics = plan.diagnostics().to_vec();
            envelope.metrics = Some(EnvelopeMetrics {
                registry_cache: freshness.as_str().into(),
                output_bytes: 0,
                compressor: "none".into(),
                gain: None,
            });
            let _ = explanation;
            emit_json(&envelope)?;
        } else {
            writeln!(io::stdout(), "{}", render_human_summary(explanation))?;
            writeln!(
                io::stdout(),
                "{}",
                render_human_policy_summary(policy_explanation)
            )?;
            writeln!(io::stdout(), "Plan only — no process started")?;
        }
        Ok(crate::exit_codes::SUCCESS)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_explanation_with_policy(
        &self,
        command: &str,
        authorized: &AuthorizedExecutionPlan,
        explanation: &ResolutionExplanation,
        freshness: Freshness,
    ) -> io::Result<u8> {
        let plan = authorized.plan();
        let policy_decision = authorized.policy_decision();
        let policy_explanation = authorized.policy_explanation();
        let execution_requested = authorized.request().execute_requested;
        let execution_authorized = true;
        if self.json {
            let data = serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
                "policy_decision": policy_decision,
                "policy": {
                    "request": policy_explanation.request,
                    "child": policy_explanation.child,
                    "execution_authorized": execution_authorized,
                    "approval_transport": policy_explanation.approval_transport,
                },
                "execution_authorized": execution_authorized,
                "execution_requested": execution_requested,
            });
            let mut envelope = CommandEnvelope::ok(command, Some(data));
            envelope.session_id = Some(plan.resolved().session_id.clone());
            let mut expl = serde_json::to_value(explanation).unwrap_or_default();
            if let Some(obj) = expl.as_object_mut() {
                obj.insert(
                    "policy".into(),
                    serde_json::to_value(policy_explanation).unwrap_or_default(),
                );
            }
            envelope.explanation = Some(expl);
            envelope.diagnostics = plan.diagnostics().to_vec();
            envelope.metrics = Some(EnvelopeMetrics {
                registry_cache: freshness.as_str().into(),
                output_bytes: 0,
                compressor: "none".into(),
                gain: None,
            });
            emit_json(&envelope)?;
        } else {
            writeln!(io::stdout(), "{}", render_human_explanation(explanation))?;
            writeln!(
                io::stdout(),
                "{}",
                render_human_policy_section(policy_explanation)
            )?;
            writeln!(io::stdout(), "Plan only — no process started")?;
        }
        Ok(crate::exit_codes::SUCCESS)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_policy_outcome(
        &self,
        command: &str,
        error: &ControllerError,
        rejected: &RejectedPolicyOutcome,
        freshness: Freshness,
    ) -> io::Result<u8> {
        let plan = rejected.plan();
        let policy_explanation = rejected.explanation();
        let policy_decision = rejected.decision();
        let execution_requested = rejected.execution_requested();
        let code = error.exit_code();
        if self.json {
            let mut envelope =
                CommandEnvelope::error(command, code, error.diagnostic_code(), &error.to_string());
            envelope.session_id = error.session_id().map(str::to_string);
            envelope.data = Some(serde_json::json!({
                "mode": "plan_only",
                "plan": PolicyFailurePlanSummary::from_plan(plan),
                "plan_digest": plan.plan_digest(),
                "policy_decision": policy_decision,
                "policy": {
                    "request": policy_explanation.request,
                    "child": policy_explanation.child,
                    "execution_authorized": false,
                    "approval_transport": policy_explanation.approval_transport,
                },
                "execution_authorized": false,
                "execution_requested": execution_requested,
            }));
            envelope.explanation = Some(serde_json::json!({
                "policy": policy_explanation,
            }));
            envelope.metrics = Some(EnvelopeMetrics {
                registry_cache: freshness.as_str().into(),
                output_bytes: 0,
                compressor: "none".into(),
                gain: None,
            });
            emit_json(&envelope)?;
        } else {
            let prefix = if self.no_color {
                "error".to_string()
            } else {
                "\x1b[31merror\x1b[0m".to_string()
            };
            writeln!(io::stderr(), "{prefix}: {error}")?;
            if let Some(sid) = error.session_id() {
                writeln!(io::stderr(), "session: {sid}")?;
            }
            writeln!(io::stderr(), "plan digest: {}", plan.plan_digest())?;
            writeln!(
                io::stderr(),
                "unit: {}  verb: {}  program: {} (arguments redacted)",
                plan.resolved().unit_id,
                plan.resolved().verb,
                PolicyFailurePlanSummary::from_plan(plan).program_basename,
            )?;
            writeln!(
                io::stderr(),
                "{}",
                render_human_policy_section(policy_explanation)
            )?;
        }
        Ok(code)
    }

    pub fn emit_policy_contract_outcome(
        &self,
        command: &str,
        error: &ControllerError,
        plan: &SealedExecutionPlan,
        freshness: Freshness,
        execution_requested: bool,
    ) -> io::Result<u8> {
        let code = error.exit_code();
        if self.json {
            let mut envelope =
                CommandEnvelope::error(command, code, error.diagnostic_code(), &error.to_string());
            envelope.session_id = error.session_id().map(str::to_string);
            let (policy_id, field) = match error {
                ControllerError::PolicyContract { details, .. } => {
                    (details.policy_id.as_deref(), details.field.as_deref())
                }
                _ => (None, None),
            };
            envelope.data = Some(serde_json::json!({
                "mode": "plan_only",
                "plan": PolicyFailurePlanSummary::from_plan(plan),
                "plan_digest": plan.plan_digest(),
                "policy_id": policy_id,
                "field": field,
                "execution_authorized": false,
                "execution_requested": execution_requested,
            }));
            envelope.metrics = Some(EnvelopeMetrics {
                registry_cache: freshness.as_str().into(),
                output_bytes: 0,
                compressor: "none".into(),
                gain: None,
            });
            emit_json(&envelope)?;
        } else {
            let prefix = if self.no_color {
                "error".to_string()
            } else {
                "\x1b[31merror\x1b[0m".to_string()
            };
            writeln!(io::stderr(), "{prefix}: {error}")?;
            if let Some(sid) = error.session_id() {
                writeln!(io::stderr(), "session: {sid}")?;
            }
            writeln!(io::stderr(), "plan digest: {}", plan.plan_digest())?;
            writeln!(
                io::stderr(),
                "unit: {}  verb: {}  program: {} (arguments redacted)",
                plan.resolved().unit_id,
                plan.resolved().verb,
                PolicyFailurePlanSummary::from_plan(plan).program_basename,
            )?;
        }
        Ok(code)
    }

    pub fn emit_error(&self, command: &str, error: &ControllerError) -> io::Result<u8> {
        self.emit_error_with_explanation(command, error, None, None)
    }

    pub fn emit_error_with_explanation(
        &self,
        command: &str,
        error: &ControllerError,
        explanation: Option<&ResolutionExplanation>,
        freshness: Option<Freshness>,
    ) -> io::Result<u8> {
        let code = error.exit_code();
        if self.json {
            let mut envelope =
                CommandEnvelope::error(command, code, error.diagnostic_code(), &error.to_string());
            envelope.session_id = error.session_id().map(str::to_string);
            if let ControllerError::Resolution {
                explanation_partial: Some(partial),
                ..
            } = error
            {
                envelope.explanation = Some(partial.as_ref().clone());
            } else if let Some(ex) = explanation {
                let mut expl = serde_json::to_value(ex).unwrap_or_default();
                if let Some(policy_ex) = policy_explanation_from_error(error)
                    && let Some(obj) = expl.as_object_mut()
                {
                    obj.insert(
                        "policy".into(),
                        serde_json::to_value(policy_ex).unwrap_or_default(),
                    );
                }
                envelope.explanation = Some(expl);
            }
            if let Some(f) = freshness {
                envelope.metrics = Some(EnvelopeMetrics {
                    registry_cache: f.as_str().into(),
                    output_bytes: 0,
                    compressor: "none".into(),
                    gain: None,
                });
            }
            match error {
                ControllerError::ExecutionUnavailable { details, .. }
                | ControllerError::ExecutionClassUnavailable { details, .. } => {
                    let authorized = true;
                    let mut data = serde_json::json!({
                        "mode": "plan_only",
                        "session_id": details.session_id(),
                        "plan_digest": details.plan_digest(),
                        "policy_decision": details.policy_decision(),
                        "execution_authorized": authorized,
                        "execution_requested": details.execution_requested(),
                    });
                    let policy_ex = details.policy_explanation();
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert(
                            "policy".into(),
                            serde_json::json!({
                                "request": policy_ex.request,
                                "child": policy_ex.child,
                                "execution_authorized": authorized,
                                "approval_transport": policy_ex.approval_transport,
                            }),
                        );
                    }
                    envelope.data = Some(data);
                }
                ControllerError::PolicyContract { details, .. } => {
                    envelope.data = Some(serde_json::json!({
                        "mode": "plan_only",
                        "session_id": details.session_id,
                        "plan_digest": details.plan_digest,
                        "policy_id": details.policy_id,
                        "field": details.field,
                    }));
                }
                _ => {}
            }
            emit_json(&envelope)?;
        } else {
            let prefix = if self.no_color {
                "error".to_string()
            } else {
                "\x1b[31merror\x1b[0m".to_string()
            };
            writeln!(io::stderr(), "{prefix}: {error}")?;
            if let Some(sid) = error.session_id() {
                writeln!(io::stderr(), "session: {sid}")?;
            }
            if let ControllerError::Resolution {
                explanation_partial: Some(partial),
                ..
            } = error
                && let Ok(trace) =
                    serde_json::from_value::<PartialResolutionTrace>(partial.as_ref().clone())
            {
                writeln!(io::stderr(), "{}", render_human_partial_explanation(&trace))?;
            } else if let Some(ex) = explanation {
                writeln!(io::stderr(), "{}", render_human_explanation(ex))?;
            }
            if let Some(policy_ex) = policy_explanation_from_error(error) {
                writeln!(io::stderr(), "{}", render_human_policy_section(policy_ex))?;
            }
        }
        Ok(code)
    }

    pub fn emit_doctor(&self, report: &DoctorReport) -> io::Result<u8> {
        if self.json {
            let mut envelope = CommandEnvelope::ok(
                "doctor",
                Some(serde_json::to_value(report).unwrap_or_default()),
            );
            if !report.ready {
                envelope.status = "error".to_string();
                envelope.exit_code = crate::exit_codes::INTERNAL;
            }
            emit_json(&envelope)?;
        } else {
            writeln!(io::stdout(), "takogami doctor (controller readiness)")?;
            writeln!(
                io::stdout(),
                "scope: required build tools + registry contracts + state-home writability"
            )?;
            for check in &report.checks {
                let mark = if check.ok { "ok" } else { "fail" };
                writeln!(
                    io::stdout(),
                    "  [{mark}] {} ({}) — {}",
                    check.name,
                    check.severity,
                    check.detail
                )?;
            }
            writeln!(
                io::stdout(),
                "status: {}",
                if report.ready {
                    "ready"
                } else {
                    "not ready (required checks failed)"
                }
            )?;
        }
        Ok(if report.ready {
            crate::exit_codes::SUCCESS
        } else {
            crate::exit_codes::INTERNAL
        })
    }
}

fn policy_explanation_from_error(error: &ControllerError) -> Option<&PolicyEvaluationExplanation> {
    match error {
        ControllerError::PolicyDeny { details, .. }
        | ControllerError::PolicyGate { details, .. } => Some(details.explanation()),
        ControllerError::ExecutionUnavailable { details, .. }
        | ControllerError::ExecutionClassUnavailable { details, .. } => {
            Some(details.policy_explanation())
        }
        _ => None,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_severity() -> String {
    "required".into()
}

mod envelope;

pub use crate::contracts::{CommandEnvelope, DiagnosticRecord, EnvelopeMetrics};
pub use envelope::emit_json;

use crate::contracts::PolicyDecision;
use crate::doctor::DoctorReport;
use crate::error::ControllerError;
use crate::policy::{
    PolicyEvaluationExplanation, render_human_policy_section, render_human_policy_summary,
};
use crate::registry::Freshness;
use crate::resolution::{
    PartialResolutionTrace, ResolutionExplanation, SealedExecutionPlan, render_human_explanation,
    render_human_partial_explanation, render_human_summary,
};
use std::io::{self, Write};

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

    pub fn emit_plan(
        &self,
        command: &str,
        plan: &SealedExecutionPlan,
        explanation: &ResolutionExplanation,
        freshness: Freshness,
    ) -> io::Result<u8> {
        let decision = dummy_allow();
        let policy_ex = dummy_policy_explanation(plan);
        self.emit_plan_with_policy(command, plan, explanation, &decision, &policy_ex, freshness)
    }

    pub fn emit_plan_with_policy(
        &self,
        command: &str,
        plan: &SealedExecutionPlan,
        explanation: &ResolutionExplanation,
        policy_decision: &PolicyDecision,
        policy_explanation: &PolicyEvaluationExplanation,
        freshness: Freshness,
    ) -> io::Result<u8> {
        if self.json {
            let data = serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
                "policy_decision": policy_decision,
                "execution_authorized": true,
                "execution_requested": false,
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
            // plan-only JSON without --explain keeps explanation absent per prior behavior,
            // but policy lives under data; keep explanation None for non-explain plan.
            envelope.diagnostics = plan.diagnostics().to_vec();
            envelope.metrics = Some(EnvelopeMetrics {
                registry_cache: freshness.as_str().into(),
                output_bytes: 0,
                compressor: "none".into(),
                gain: None,
            });
            let _ = expl;
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

    pub fn emit_explanation(
        &self,
        command: &str,
        plan: &SealedExecutionPlan,
        explanation: &ResolutionExplanation,
        freshness: Freshness,
    ) -> io::Result<u8> {
        self.emit_explanation_with_policy(
            command,
            plan,
            explanation,
            &dummy_allow(),
            &dummy_policy_explanation(plan),
            freshness,
        )
    }

    pub fn emit_explanation_with_policy(
        &self,
        command: &str,
        plan: &SealedExecutionPlan,
        explanation: &ResolutionExplanation,
        policy_decision: &PolicyDecision,
        policy_explanation: &PolicyEvaluationExplanation,
        freshness: Freshness,
    ) -> io::Result<u8> {
        if self.json {
            let data = serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
                "policy_decision": policy_decision,
                "execution_authorized": true,
                "execution_requested": false,
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
        plan: &SealedExecutionPlan,
        resolution_explanation: &ResolutionExplanation,
        policy_explanation: &PolicyEvaluationExplanation,
        policy_decision: &PolicyDecision,
        freshness: Freshness,
    ) -> io::Result<u8> {
        let code = error.exit_code();
        if self.json {
            let mut envelope =
                CommandEnvelope::error(command, code, error.diagnostic_code(), &error.to_string());
            envelope.session_id = error.session_id().map(str::to_string);
            envelope.data = Some(serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
                "policy_decision": policy_decision,
                "execution_authorized": false,
                "execution_requested": false,
            }));
            let mut expl = serde_json::to_value(resolution_explanation).unwrap_or_default();
            if let Some(obj) = expl.as_object_mut() {
                obj.insert(
                    "policy".into(),
                    serde_json::to_value(policy_explanation).unwrap_or_default(),
                );
            }
            envelope.explanation = Some(expl);
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
            writeln!(
                io::stderr(),
                "{}",
                render_human_policy_section(policy_explanation)
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
                    envelope.data = Some(serde_json::json!({
                        "mode": "plan_only",
                        "session_id": details.session_id,
                        "plan_digest": details.plan_digest,
                        "policy_decision": details.policy_decision,
                        "execution_authorized": details.policy_decision.is_some(),
                        "execution_requested": true,
                    }));
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
        | ControllerError::PolicyGate { details, .. } => Some(&details.explanation),
        ControllerError::ExecutionUnavailable { details, .. }
        | ControllerError::ExecutionClassUnavailable { details, .. } => {
            details.policy_explanation.as_ref()
        }
        _ => None,
    }
}

fn dummy_allow() -> PolicyDecision {
    PolicyDecision::Allow {
        matched_rules: vec![],
    }
}

fn dummy_policy_explanation(plan: &SealedExecutionPlan) -> PolicyEvaluationExplanation {
    use crate::policy::{PolicyLayer, PolicyLayerResult};
    PolicyEvaluationExplanation {
        actor: "agent".into(),
        profile_id: plan.resolved().profile_id.clone(),
        plan_digest: plan.plan_digest().to_string(),
        precedence: "deny>gate>allow".into(),
        request: PolicyLayerResult {
            layer: PolicyLayer::Request,
            decision: "allow".into(),
            matched_rules: vec![],
            primary_rule: None,
            intents: vec![],
        },
        child: PolicyLayerResult {
            layer: PolicyLayer::Child,
            decision: "allow".into(),
            matched_rules: vec![],
            primary_rule: None,
            intents: vec![],
        },
        effective_decision: dummy_allow(),
        execution_authorized: true,
        approval_transport: "unavailable".into(),
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

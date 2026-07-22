mod envelope;

pub use crate::contracts::{CommandEnvelope, DiagnosticRecord, EnvelopeMetrics};
pub use envelope::emit_json;

use crate::doctor::DoctorReport;
use crate::error::ControllerError;
use crate::registry::Freshness;
use crate::resolution::{
    ResolutionExplanation, SealedExecutionPlan, render_human_explanation, render_human_summary,
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
        if self.json {
            let data = serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
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
            emit_json(&envelope)?;
        } else {
            writeln!(io::stdout(), "{}", render_human_summary(explanation))?;
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
        if self.json {
            let data = serde_json::json!({
                "mode": "plan_only",
                "resolved_command": plan.resolved(),
                "plan_digest": plan.plan_digest(),
            });
            let mut envelope = CommandEnvelope::ok(command, Some(data));
            envelope.session_id = Some(plan.resolved().session_id.clone());
            envelope.explanation = Some(serde_json::to_value(explanation).unwrap_or_default());
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
        }
        Ok(crate::exit_codes::SUCCESS)
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
                envelope.explanation = Some(partial.clone());
            } else if let Some(ex) = explanation {
                envelope.explanation = Some(serde_json::to_value(ex).unwrap_or_default());
            }
            if let Some(f) = freshness {
                envelope.metrics = Some(EnvelopeMetrics {
                    registry_cache: f.as_str().into(),
                    output_bytes: 0,
                    compressor: "none".into(),
                    gain: None,
                });
            }
            // Attach plan digest in data for execution_* errors when available.
            match error {
                ControllerError::ExecutionUnavailable {
                    plan_digest,
                    session_id,
                }
                | ControllerError::ExecutionClassUnavailable {
                    plan_digest,
                    session_id,
                    ..
                } => {
                    envelope.data = Some(serde_json::json!({
                        "mode": "plan_only",
                        "session_id": session_id,
                        "plan_digest": plan_digest,
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
            if let Some(ex) = explanation {
                writeln!(io::stderr(), "{}", render_human_explanation(ex))?;
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

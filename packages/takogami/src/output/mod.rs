mod envelope;

pub use crate::contracts::{CommandEnvelope, DiagnosticRecord, EnvelopeMetrics};
pub use envelope::emit_json;

use crate::doctor::DoctorReport;
use crate::error::ControllerError;
use crate::registry::Freshness;
use std::io::{self, Write};

/// Write command output to stdout; diagnostics go to stderr when not JSON.
pub struct OutputSink {
    pub json: bool,
    pub no_color: bool,
}

impl OutputSink {
    pub fn emit_envelope(&self, envelope: &CommandEnvelope) -> io::Result<()> {
        if self.json {
            emit_json(envelope)
        } else {
            // Human mode for structured data is handled by callers.
            emit_json(envelope)
        }
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

    pub fn emit_error(&self, command: &str, error: &ControllerError) -> io::Result<u8> {
        let code = error.exit_code();
        if self.json {
            let envelope =
                CommandEnvelope::error(command, code, error.diagnostic_code(), &error.to_string());
            emit_json(&envelope)?;
        } else {
            let prefix = if self.no_color {
                "error".to_string()
            } else {
                "\x1b[31merror\x1b[0m".to_string()
            };
            writeln!(io::stderr(), "{prefix}: {error}")?;
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

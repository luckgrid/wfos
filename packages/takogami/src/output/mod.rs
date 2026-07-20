mod envelope;

pub use crate::contracts::{CommandEnvelope, DiagnosticRecord};
pub use envelope::emit_json;

use crate::error::ControllerError;
use std::io::{self, Write};

/// Write command output to stdout; diagnostics go to stderr when not JSON.
pub struct OutputSink {
    pub json: bool,
    pub no_color: bool,
}

impl OutputSink {
    pub fn emit_success_json(&self, envelope: &CommandEnvelope) -> io::Result<()> {
        emit_json(envelope)
    }

    pub fn emit_error(&self, command: &str, error: &ControllerError) -> io::Result<u8> {
        let code = error.exit_code();
        if self.json {
            let envelope =
                CommandEnvelope::error(command, code, error.diagnostic_code(), &error.to_string());
            self.emit_success_json(&envelope)?;
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

    pub fn emit_doctor_human(&self, checks: &[DoctorCheck], ready: bool) -> io::Result<()> {
        writeln!(io::stdout(), "takogami doctor (build toolchain skeleton)")?;
        writeln!(
            io::stdout(),
            "scope: controller build tools only — registry/session/RTK checks arrive in S3"
        )?;
        for check in checks {
            let mark = if check.ok { "ok" } else { "missing" };
            writeln!(io::stdout(), "  [{mark}] {} — {}", check.name, check.detail)?;
        }
        writeln!(
            io::stdout(),
            "status: {}",
            if ready {
                "ready (skeleton)"
            } else {
                "missing required tools"
            }
        )?;
        Ok(())
    }

    pub fn emit_doctor_json(&self, checks: &[DoctorCheck], ready: bool) -> io::Result<()> {
        let data = DoctorReport {
            scope: "build_toolchain_skeleton",
            registry_readiness: false,
            session_readiness: false,
            rtk_readiness: false,
            ready,
            checks: checks.to_vec(),
        };
        let mut envelope = CommandEnvelope::ok(
            "doctor",
            Some(serde_json::to_value(data).unwrap_or_default()),
        );
        if !ready {
            envelope.status = "error".to_string();
            envelope.exit_code = crate::exit_codes::INTERNAL;
        }
        self.emit_success_json(&envelope)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DoctorReport {
    scope: &'static str,
    registry_readiness: bool,
    session_readiness: bool,
    rtk_readiness: bool,
    ready: bool,
    checks: Vec<DoctorCheck>,
}

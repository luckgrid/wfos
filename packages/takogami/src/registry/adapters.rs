//! External process adapters (Ontarch / Panoply) — literal argv, no shell.

use std::path::Path;
use std::process::{Command, Output};

use crate::error::ControllerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshKind {
    Sync,
    Scan,
}

pub trait ExternalAdapters {
    fn refresh(&self, kind: RefreshKind, cwd: &Path) -> Result<Output, ControllerError>;
    fn validate(&self, cwd: &Path) -> Result<Output, ControllerError>;
    fn panoply_doctor_json(&self) -> Result<Output, ControllerError>;
    fn panoply_env_json(&self) -> Result<Output, ControllerError>;
}

/// Real adapters: `moon run ontarch:…` and `panoply … --json`.
#[derive(Debug, Default, Clone)]
pub struct ProcessAdapters;

impl ExternalAdapters for ProcessAdapters {
    fn refresh(&self, kind: RefreshKind, cwd: &Path) -> Result<Output, ControllerError> {
        let task = match kind {
            RefreshKind::Sync => "ontarch:sync",
            RefreshKind::Scan => "ontarch:scan",
        };
        run_literal(cwd, "moon", &["run", task])
    }

    fn validate(&self, cwd: &Path) -> Result<Output, ControllerError> {
        run_literal(cwd, "moon", &["run", "ontarch:validate"])
    }

    fn panoply_doctor_json(&self) -> Result<Output, ControllerError> {
        run_literal(Path::new("."), "panoply", &["doctor", "--json"])
    }

    fn panoply_env_json(&self) -> Result<Output, ControllerError> {
        run_literal(Path::new("."), "panoply", &["env", "--json"])
    }
}

fn run_literal(cwd: &Path, program: &str, args: &[&str]) -> Result<Output, ControllerError> {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| {
            ControllerError::unavailable_source(format!(
                "failed to spawn `{program} {}`: {e}",
                args.join(" ")
            ))
        })
}

/// Test double that records calls and returns canned stdout.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct MockAdapters {
    pub refresh_calls: std::sync::Mutex<Vec<RefreshKind>>,
    pub validate_calls: std::sync::Mutex<u32>,
    pub refresh_ok: bool,
    pub validate_ok: bool,
    pub panoply_doctor_stdout: String,
    pub panoply_env_stdout: String,
}

#[allow(dead_code)]
impl ExternalAdapters for MockAdapters {
    fn refresh(&self, kind: RefreshKind, _cwd: &Path) -> Result<Output, ControllerError> {
        self.refresh_calls.lock().unwrap().push(kind);
        Ok(fake_output(self.refresh_ok, b"ok\n"))
    }

    fn validate(&self, _cwd: &Path) -> Result<Output, ControllerError> {
        *self.validate_calls.lock().unwrap() += 1;
        Ok(fake_output(self.validate_ok, b"ok\n"))
    }

    fn panoply_doctor_json(&self) -> Result<Output, ControllerError> {
        Ok(fake_output(true, self.panoply_doctor_stdout.as_bytes()))
    }

    fn panoply_env_json(&self) -> Result<Output, ControllerError> {
        Ok(fake_output(true, self.panoply_env_stdout.as_bytes()))
    }
}

#[allow(dead_code)]
fn fake_output(success: bool, stdout: &[u8]) -> Output {
    let status = if success {
        Command::new("true").status().expect("true")
    } else {
        Command::new("false").status().expect("false")
    };
    Output {
        status,
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    }
}

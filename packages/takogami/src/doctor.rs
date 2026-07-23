//! Doctor readiness checks (extends PATH skeleton).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::contracts::{StateHomeInputs, resolve_session_state_home};
use crate::output::DoctorCheck;
use crate::registry::{RegistryAccess, RegistryPaths};

const REQUIRED_TOOLS: &[&str] = &["cargo", "rustc", "moon"];
const OPTIONAL_TOOLS: &[&str] = &["rtk", "tmux", "herdr"];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorReport {
    pub scope: String,
    pub registry_readiness: bool,
    pub session_readiness: bool,
    pub rtk_readiness: bool,
    pub ready: bool,
    pub checks: Vec<DoctorCheck>,
}

pub struct DoctorInputs<'a> {
    pub registry: Option<&'a RegistryAccess>,
    pub cli_state_home: Option<&'a Path>,
    pub path_var: Option<&'a str>,
}

pub fn run_doctor(inputs: DoctorInputs<'_>) -> DoctorReport {
    let mut checks = Vec::new();
    let mut ready = true;
    let path = inputs
        .path_var
        .map(str::to_string)
        .unwrap_or_else(|| env::var("PATH").unwrap_or_default());

    for tool in REQUIRED_TOOLS {
        let found = which_in_path(tool, &path);
        if !found {
            ready = false;
        }
        checks.push(DoctorCheck {
            name: (*tool).to_string(),
            ok: found,
            detail: if found {
                "found on PATH (required)".to_string()
            } else {
                "not found on PATH (required)".to_string()
            },
            severity: "required".into(),
        });
    }

    let (registry_ok, registry_detail) = match inputs.registry {
        Some(reg) => reg.contracts_readable(),
        None => (false, "registry access not configured".into()),
    };
    if !registry_ok {
        ready = false;
    }
    checks.push(DoctorCheck {
        name: "registry_contracts".into(),
        ok: registry_ok,
        detail: registry_detail,
        severity: "required".into(),
    });

    let env_state = env::var("TAKOGAMI_STATE_HOME").ok();
    let env_xdg = env::var("XDG_STATE_HOME").ok();
    let home = env::var_os("HOME").map(PathBuf::from);
    let state_home = resolve_session_state_home(StateHomeInputs {
        cli_state_home: inputs.cli_state_home,
        env_takogami_state_home: env_state.as_deref(),
        profile_session_state_home: None,
        env_xdg_state_home: env_xdg.as_deref(),
        home_dir: home.as_deref(),
    });
    let (state_ok, state_detail) = check_state_home_writable(&state_home);
    if !state_ok {
        ready = false;
    }
    checks.push(DoctorCheck {
        name: "state_home_writable".into(),
        ok: state_ok,
        detail: state_detail,
        severity: "required".into(),
    });

    let mut rtk_ok = false;
    for tool in OPTIONAL_TOOLS {
        let found = which_in_path(tool, &path);
        if *tool == "rtk" {
            rtk_ok = found;
        }
        checks.push(DoctorCheck {
            name: (*tool).to_string(),
            ok: found,
            detail: if found {
                "found on PATH (optional)".to_string()
            } else {
                "not found on PATH (optional — not required)".to_string()
            },
            severity: "optional".into(),
        });
    }

    DoctorReport {
        scope: "controller_readiness".into(),
        registry_readiness: registry_ok,
        session_readiness: false, // operational command records land in S6
        rtk_readiness: rtk_ok,
        ready,
        checks,
    }
}

/// Probe writability without persisting a command execution record.
fn check_state_home_writable(state_home: &Path) -> (bool, String) {
    if let Err(e) = fs::create_dir_all(state_home) {
        return (
            false,
            format!("cannot create {}: {e}", state_home.display()),
        );
    }
    let probe = state_home.join(".takogami-doctor-probe");
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            (
                true,
                format!(
                    "writable at {} (probe removed; no session record)",
                    state_home.display()
                ),
            )
        }
        Err(e) => (
            false,
            format!("cannot write probe under {}: {e}", state_home.display()),
        ),
    }
}

fn which_in_path(name: &str, path_var: &str) -> bool {
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return true;
        }
    }
    false
}

pub fn default_registry_for_doctor() -> Option<RegistryAccess> {
    resolve_registry_paths_quiet().map(RegistryAccess::new)
}

fn resolve_registry_paths_quiet() -> Option<RegistryPaths> {
    crate::registry::resolve_registry_paths().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_detects_injected_path() {
        let temp = tempfile::tempdir().unwrap();
        let fake = temp.path().join("cargo");
        std::fs::write(&fake, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:/usr/bin", temp.path().display());
        assert!(which_in_path("cargo", &path));
    }

    #[test]
    fn missing_herdr_does_not_fail_ready_when_required_ok() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["cargo", "rustc", "moon"] {
            let p = temp.path().join(name);
            std::fs::write(&p, b"").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let reg_root = temp.path().join("registry");
        fs::create_dir_all(&reg_root).unwrap();
        fs::write(
            reg_root.join("units.json"),
            r#"{"generated_at":"t","summary":{},"units":[]}"#,
        )
        .unwrap();
        let access = RegistryAccess::new(RegistryPaths {
            registry_root: reg_root,
            workspace_root: temp.path().to_path_buf(),
        });
        let state = temp.path().join("state");
        let report = run_doctor(DoctorInputs {
            registry: Some(&access),
            cli_state_home: Some(&state),
            path_var: Some(&format!("{}:/usr/bin", temp.path().display())),
        });
        assert!(report.ready);
        let herdr = report
            .checks
            .iter()
            .find(|c| c.name == "herdr")
            .expect("herdr check");
        assert!(!herdr.ok);
        assert_eq!(herdr.severity, "optional");
    }
}

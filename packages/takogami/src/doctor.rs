use crate::output::DoctorCheck;
use std::env;

const REQUIRED_TOOLS: &[&str] = &["cargo", "rustc", "moon"];

pub fn run_doctor() -> (Vec<DoctorCheck>, bool) {
    let mut checks = Vec::new();
    let mut ready = true;

    for tool in REQUIRED_TOOLS {
        let path = env::var("PATH").unwrap_or_default();
        let found = which_in_path(tool, &path);
        if !found {
            ready = false;
        }
        checks.push(DoctorCheck {
            name: (*tool).to_string(),
            ok: found,
            detail: if found {
                "found on PATH".to_string()
            } else {
                "not found on PATH".to_string()
            },
        });
    }

    (checks, ready)
}

fn which_in_path(name: &str, path_var: &str) -> bool {
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let candidate = std::path::Path::new(dir).join(name);
        if candidate.is_file() {
            return true;
        }
    }
    false
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
}

//! Operational command-record state-home resolution.
//!
//! Precedence: `--state-home` → `TAKOGAMI_STATE_HOME` → profile `[runtime] session_state_home`
//! → `$XDG_STATE_HOME/takogami/sessions` → `~/.local/state/takogami/sessions`.
//!
//! `logs.session_log_target` is tracked build-session provenance and must never be used here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Debug, Clone, Default)]
pub struct StateHomeInputs<'a> {
    pub cli_state_home: Option<&'a Path>,
    pub env_takogami_state_home: Option<&'a str>,
    pub profile_session_state_home: Option<&'a str>,
    pub env_xdg_state_home: Option<&'a str>,
    pub home_dir: Option<&'a Path>,
}

/// Resolve the operational command-record directory (MVP path still ends in `sessions`).
pub fn resolve_session_state_home(inputs: StateHomeInputs<'_>) -> PathBuf {
    if let Some(path) = inputs.cli_state_home {
        return path.to_path_buf();
    }
    if let Some(path) = inputs.env_takogami_state_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = inputs.profile_session_state_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(xdg) = inputs.env_xdg_state_home.filter(|s| !s.is_empty()) {
        return PathBuf::from(xdg).join("takogami").join("sessions");
    }
    let home = inputs
        .home_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    home.join(".local")
        .join("state")
        .join("takogami")
        .join("sessions")
}

/// Reject existing non-directory or symlink state roots.
pub fn validate_state_home(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "state home must not be a symlink",
                ));
            }
            if !meta.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "state home exists but is not a directory",
                ));
            }
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Create the operational directory tree when a record or query needs it.
pub fn ensure_state_home(path: &Path) -> io::Result<()> {
    validate_state_home(path)?;
    if !path.exists() {
        fs::create_dir_all(path)?;
        set_dir_mode(path)?;
    }
    for sub in [".locks", ".tmp"] {
        let child = path.join(sub);
        if !child.exists() {
            fs::create_dir_all(&child)?;
            set_dir_mode(&child)?;
        } else {
            validate_state_home(&child)?;
            set_dir_mode(&child)?;
        }
    }
    set_dir_mode(path)?;
    Ok(())
}

fn set_dir_mode(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(path, perms)?;
    }
    let _ = path;
    Ok(())
}

pub(crate) fn set_file_mode(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    let _ = path;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn open_new_private(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
pub(crate) fn open_new_private(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn cli_wins() {
        let cli = Path::new("/tmp/cli-state");
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: Some(cli),
            env_takogami_state_home: Some("/tmp/env"),
            profile_session_state_home: Some("/tmp/profile"),
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, cli);
    }

    #[test]
    fn env_before_profile() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: Some("/tmp/env"),
            profile_session_state_home: Some("/tmp/profile"),
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, PathBuf::from("/tmp/env"));
    }

    #[test]
    fn profile_before_xdg() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: None,
            profile_session_state_home: Some("/tmp/profile"),
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, PathBuf::from("/tmp/profile"));
    }

    #[test]
    fn xdg_before_home_fallback() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: None,
            profile_session_state_home: None,
            env_xdg_state_home: Some("/tmp/xdg"),
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(got, PathBuf::from("/tmp/xdg/takogami/sessions"));
    }

    #[test]
    fn home_fallback() {
        let got = resolve_session_state_home(StateHomeInputs {
            cli_state_home: None,
            env_takogami_state_home: None,
            profile_session_state_home: None,
            env_xdg_state_home: None,
            home_dir: Some(Path::new("/Users/x")),
        });
        assert_eq!(
            got,
            PathBuf::from("/Users/x/.local/state/takogami/sessions")
        );
    }

    #[test]
    fn ensure_state_home_rejects_symlink_root() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("real");
        let link = temp.path().join("link");
        fs::create_dir(&target).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let err = ensure_state_home(&link).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
    }
}

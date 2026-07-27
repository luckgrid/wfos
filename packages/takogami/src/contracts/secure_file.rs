//! Race-safe no-follow regular-file open, bounded read, and streaming hash.
//!
//! Shared by graph I/O and projection source fingerprinting. Errors use logical
//! display labels only — never physical absolute roots.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Operation that failed during a secure file access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecureFileOperation {
    Open,
    Fstat,
    Read,
    Hash,
}

impl SecureFileOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Fstat => "fstat",
            Self::Read => "read",
            Self::Hash => "hash",
        }
    }
}

/// Typed secure-file failure. Physical paths stay out of Display/mapping text.
#[derive(Debug)]
pub(crate) enum SecureFileError {
    Missing,
    Symlink,
    NonRegular,
    Limit {
        limit: u64,
    },
    Io {
        operation: SecureFileOperation,
        display: String,
        #[allow(dead_code)]
        source: std::io::Error,
    },
}

impl SecureFileError {
    pub(crate) fn io(
        operation: SecureFileOperation,
        display: &str,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            operation,
            display: display.to_string(),
            source,
        }
    }

    /// Stable public message fragment using the logical display label only.
    pub(crate) fn public_message(&self) -> String {
        match self {
            Self::Missing => "missing path".into(),
            Self::Symlink => "path must not be a symlink".into(),
            Self::NonRegular => "path must be a regular non-symlink file".into(),
            Self::Limit { limit } => format!("exceeds {limit} byte limit"),
            Self::Io {
                operation, display, ..
            } => {
                format!("cannot {} {display}", operation.as_str())
            }
        }
    }
}

#[cfg(test)]
type OpenOverrideFn = fn(&Path, &str) -> Result<fs::File, SecureFileError>;

#[cfg(test)]
thread_local! {
    // Test seam: when set, replaces only the open step; post-open fstat still runs.
    static OPEN_OVERRIDE: std::cell::Cell<Option<OpenOverrideFn>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_open_override(f: Option<OpenOverrideFn>) {
    OPEN_OVERRIDE.with(|c| c.set(f));
}

/// Open a path read-only without following symlinks; require a regular file.
///
/// Unix flags: `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`.
pub(crate) fn open_regular_nofollow(
    physical: &Path,
    display: &str,
) -> Result<fs::File, SecureFileError> {
    #[cfg(test)]
    let file = if let Some(f) = OPEN_OVERRIDE.with(|c| c.get()) {
        f(physical, display)?
    } else {
        open_regular_nofollow_inner(physical, display)?
    };
    #[cfg(not(test))]
    let file = open_regular_nofollow_inner(physical, display)?;

    let meta = file
        .metadata()
        .map_err(|e| SecureFileError::io(SecureFileOperation::Fstat, display, e))?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        return Err(SecureFileError::Symlink);
    }
    if ft.is_dir() || !ft.is_file() {
        return Err(SecureFileError::NonRegular);
    }
    Ok(file)
}

fn open_regular_nofollow_inner(
    physical: &Path,
    display: &str,
) -> Result<fs::File, SecureFileError> {
    match std::fs::symlink_metadata(physical) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SecureFileError::Missing),
        Err(e) => return Err(SecureFileError::io(SecureFileOperation::Open, display, e)),
        Ok(meta) => {
            let ft = meta.file_type();
            if ft.is_symlink() {
                return Err(SecureFileError::Symlink);
            }
            if ft.is_dir() || !ft.is_file() {
                return Err(SecureFileError::NonRegular);
            }
        }
    }

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(physical)
        .map_err(|e| classify_open_err(physical, display, e))
}

fn classify_open_err(physical: &Path, display: &str, e: std::io::Error) -> SecureFileError {
    if e.kind() == std::io::ErrorKind::NotFound {
        return SecureFileError::Missing;
    }
    // After O_NOFOLLOW failure, re-stat without following to distinguish symlink.
    if let Ok(meta) = std::fs::symlink_metadata(physical) {
        let ft = meta.file_type();
        if ft.is_symlink() {
            return SecureFileError::Symlink;
        }
        if ft.is_dir() || !ft.is_file() {
            return SecureFileError::NonRegular;
        }
    } else if e.kind() == std::io::ErrorKind::NotFound {
        return SecureFileError::Missing;
    }
    SecureFileError::io(SecureFileOperation::Open, display, e)
}

/// Read at most `limit` bytes (+1 sentinel) from a no-follow regular file.
pub(crate) fn read_bounded_nofollow(
    physical: &Path,
    display: &str,
    limit: u64,
) -> Result<Vec<u8>, SecureFileError> {
    let mut file = open_regular_nofollow(physical, display)?;
    if let Ok(opened) = file.metadata()
        && opened.len() > limit
    {
        return Err(SecureFileError::Limit { limit });
    }
    let mut buf = Vec::new();
    let take_limit = limit.saturating_add(1);
    let mut take = (&mut file).take(take_limit);
    take.read_to_end(&mut buf)
        .map_err(|e| SecureFileError::io(SecureFileOperation::Read, display, e))?;
    if (buf.len() as u64) > limit {
        return Err(SecureFileError::Limit { limit });
    }
    Ok(buf)
}

/// Stream SHA-256 hex digest of a no-follow regular file without full-file allocation.
pub(crate) fn sha256_hex_regular_nofollow(
    physical: &Path,
    display: &str,
) -> Result<String, SecureFileError> {
    let mut file = open_regular_nofollow(physical, display)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut chunk)
            .map_err(|e| SecureFileError::io(SecureFileOperation::Hash, display, e))?;
        if n == 0 {
            break;
        }
        hasher
            .write_all(&chunk[..n])
            .map_err(|e| SecureFileError::io(SecureFileOperation::Hash, display, e))?;
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::process::Command;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn rejects_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("real");
        let link = dir.path().join("link");
        fs::write(&target, b"hi").unwrap();
        symlink(&target, &link).unwrap();
        let err = open_regular_nofollow(&link, "registry/graph.json").unwrap_err();
        assert!(matches!(err, SecureFileError::Symlink));
        assert!(!err.public_message().contains(dir.path().to_str().unwrap()));
    }

    #[test]
    fn rejects_dangling_symlink_as_symlink() {
        let dir = tempdir().unwrap();
        let link = dir.path().join("dangling");
        symlink(dir.path().join("missing-target"), &link).unwrap();
        let err = open_regular_nofollow(&link, "registry/graph.json").unwrap_err();
        assert!(matches!(err, SecureFileError::Symlink));
    }

    #[test]
    fn missing_is_typed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gone");
        let err = open_regular_nofollow(&path, "registry/graph.json").unwrap_err();
        assert!(matches!(err, SecureFileError::Missing));
    }

    #[test]
    fn hashes_regular_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, b"hello").unwrap();
        let digest = sha256_hex_regular_nofollow(&path, "f").unwrap();
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn public_messages_omit_physical_roots() {
        let dir = tempdir().unwrap();
        let abs = dir.path().join("x");
        fs::create_dir(&abs).unwrap();
        let err = open_regular_nofollow(&abs, "registry/units.json").unwrap_err();
        let msg = err.public_message();
        assert!(!msg.contains(dir.path().to_str().unwrap()));
        assert!(matches!(err, SecureFileError::NonRegular));
    }

    #[test]
    fn rejects_fifo_without_blocking() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pipe");
        let status = Command::new("/usr/bin/mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo");
        assert!(status.success());
        let start = Instant::now();
        let err = open_regular_nofollow(&path, "packages/ontarch/lib/common.sh").unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "FIFO open blocked"
        );
        assert!(matches!(err, SecureFileError::NonRegular));
    }

    #[test]
    fn post_open_nonregular_fails_closed() {
        fn open_dev_null(_physical: &Path, _display: &str) -> Result<fs::File, SecureFileError> {
            fs::File::open("/dev/null")
                .map_err(|e| SecureFileError::io(SecureFileOperation::Open, "dev-null", e))
        }
        set_open_override(Some(open_dev_null));
        let err = open_regular_nofollow(Path::new("/tmp"), "logical/source").unwrap_err();
        set_open_override(None);
        assert!(matches!(err, SecureFileError::NonRegular));
        assert!(!err.public_message().contains("/dev/null"));
    }
}

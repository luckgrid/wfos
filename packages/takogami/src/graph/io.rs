//! Race-safe no-follow regular-file reads for graph freshness and load.
//!
//! Helpers return [`SecureFileError`] with logical display labels only — never
//! put physical absolute paths into public diagnostics.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::types::GraphSourceFingerprint;

/// Operation that failed during a secure file access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SecureFileOperation {
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
pub(super) enum SecureFileError {
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
    pub(super) fn io(
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
    pub(super) fn public_message(&self) -> String {
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
thread_local! {
    // Test seam: when set, open_regular_nofollow delegates to this override.
    static OPEN_OVERRIDE: std::cell::Cell<Option<fn(&Path, &str) -> Result<fs::File, SecureFileError>>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn set_open_override(f: Option<fn(&Path, &str) -> Result<fs::File, SecureFileError>>) {
    OPEN_OVERRIDE.with(|c| c.set(f));
}

/// Open a path read-only without following symlinks; require a regular file.
///
/// Unix flags: `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`.
pub(super) fn open_regular_nofollow(
    physical: &Path,
    display: &str,
) -> Result<fs::File, SecureFileError> {
    #[cfg(test)]
    if let Some(f) = OPEN_OVERRIDE.with(|c| c.get()) {
        return f(physical, display);
    }

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

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(physical)
        .map_err(|e| classify_open_err(physical, display, e))?;

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
pub(super) fn read_bounded_nofollow(
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

/// Stream SHA-256 of a no-follow regular file without full-file allocation.
pub(super) fn sha256_regular_nofollow(
    physical: &Path,
    display: &str,
) -> Result<GraphSourceFingerprint, SecureFileError> {
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
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(GraphSourceFingerprint {
        path: display.to_string(),
        algorithm: "sha256".into(),
        digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
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
        let fp = sha256_regular_nofollow(&path, "f").unwrap();
        assert_eq!(fp.algorithm, "sha256");
        assert_eq!(fp.digest.len(), 64);
        assert_eq!(fp.path, "f");
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
}

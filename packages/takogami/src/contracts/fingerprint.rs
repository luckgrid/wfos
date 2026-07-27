//! Source fingerprint helpers (SHA-256 over raw authored bytes).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;

use super::secure_file::{SecureFileError, sha256_hex_regular_nofollow};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceFingerprint {
    pub path: String,
    pub algorithm: String,
    pub digest: String,
}

/// Fingerprint raw bytes as `sha256:<hex>`.
pub fn fingerprint_bytes(data: &[u8]) -> SourceFingerprint {
    let digest = hex_digest(data);
    SourceFingerprint {
        path: String::new(),
        algorithm: "sha256".to_string(),
        digest,
    }
}

/// Fingerprint a file's raw bytes. `path` is recorded as the display path argument.
pub fn fingerprint_file(path: &Path, display_path: &str) -> io::Result<SourceFingerprint> {
    let data = fs::read(path)?;
    let mut fp = fingerprint_bytes(&data);
    fp.path = display_path.to_string();
    Ok(fp)
}

/// Fingerprint a no-follow regular file via streaming SHA-256 on a secured descriptor.
///
/// Rejects missing paths, symlinks, directories, FIFOs, devices, and other non-regular files.
/// Uses `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK` plus post-open fstat.
/// `display_path` is the stable logical label stored on the fingerprint (never an absolute root).
pub fn fingerprint_regular_file_nofollow(
    path: &Path,
    display_path: &str,
) -> io::Result<SourceFingerprint> {
    let digest = sha256_hex_regular_nofollow(path, display_path).map_err(secure_to_io)?;
    Ok(SourceFingerprint {
        path: display_path.to_string(),
        algorithm: "sha256".to_string(),
        digest,
    })
}

fn secure_to_io(err: SecureFileError) -> io::Error {
    let kind = match &err {
        SecureFileError::Missing => io::ErrorKind::NotFound,
        SecureFileError::Symlink | SecureFileError::NonRegular | SecureFileError::Limit { .. } => {
            io::ErrorKind::InvalidInput
        }
        SecureFileError::Io { source, .. } => source.kind(),
    };
    let msg = match &err {
        SecureFileError::Symlink => "source must not be a symlink".to_string(),
        SecureFileError::NonRegular => "source must be a regular file".to_string(),
        other => other.public_message(),
    };
    io::Error::new(kind, msg)
}

fn hex_digest(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::secure_file::{SecureFileError, SecureFileOperation, set_open_override};
    use std::os::unix::fs::symlink;
    use std::process::Command;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn fingerprint_is_deterministic() {
        let a = fingerprint_bytes(b"hello");
        let b = fingerprint_bytes(b"hello");
        assert_eq!(a.digest, b.digest);
        assert_eq!(a.algorithm, "sha256");
        assert_eq!(a.digest.len(), 64);
    }

    #[test]
    fn fingerprint_differs_for_content() {
        assert_ne!(
            fingerprint_bytes(b"a").digest,
            fingerprint_bytes(b"b").digest
        );
    }

    #[test]
    fn fingerprint_rejects_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("real");
        let link = dir.path().join("link");
        fs::write(&target, b"hi").unwrap();
        symlink(&target, &link).unwrap();
        let err =
            fingerprint_regular_file_nofollow(&link, "packages/ontarch/lib/common.sh").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("symlink"));
        assert!(!err.to_string().contains(dir.path().to_str().unwrap()));
    }

    #[test]
    fn fingerprint_rejects_dangling_symlink() {
        let dir = tempdir().unwrap();
        let link = dir.path().join("dangling");
        symlink(dir.path().join("missing-target"), &link).unwrap();
        let err =
            fingerprint_regular_file_nofollow(&link, "packages/ontarch/lib/common.sh").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("symlink"));
    }

    #[test]
    fn fingerprint_rejects_directory() {
        let dir = tempdir().unwrap();
        let err =
            fingerprint_regular_file_nofollow(dir.path(), "packages/ontarch/lib").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("regular file"));
        assert!(!err.to_string().contains(dir.path().to_str().unwrap()));
    }

    #[test]
    fn fingerprint_rejects_fifo_without_blocking() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pipe");
        let status = Command::new("/usr/bin/mkfifo")
            .arg(&path)
            .status()
            .expect("mkfifo");
        assert!(status.success());
        let start = Instant::now();
        let err =
            fingerprint_regular_file_nofollow(&path, "packages/ontarch/lib/common.sh").unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "FIFO fingerprint blocked"
        );
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn fingerprint_rejects_device_or_other_nonregular() {
        let err =
            fingerprint_regular_file_nofollow(Path::new("/dev/null"), "dev-null").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(!err.to_string().contains("/dev/null"));
    }

    #[test]
    fn fingerprint_open_swap_to_symlink_fails_closed() {
        fn open_as_symlink(_physical: &Path, _display: &str) -> Result<fs::File, SecureFileError> {
            Err(SecureFileError::Symlink)
        }
        set_open_override(Some(open_as_symlink));
        let err =
            fingerprint_regular_file_nofollow(Path::new("/tmp"), "packages/ontarch/lib/common.sh")
                .unwrap_err();
        set_open_override(None);
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("symlink"));
    }

    #[test]
    fn fingerprint_open_swap_to_fifo_fails_without_blocking() {
        fn open_as_fifo(_physical: &Path, _display: &str) -> Result<fs::File, SecureFileError> {
            Err(SecureFileError::NonRegular)
        }
        set_open_override(Some(open_as_fifo));
        let start = Instant::now();
        let err =
            fingerprint_regular_file_nofollow(Path::new("/tmp"), "packages/ontarch/lib/common.sh")
                .unwrap_err();
        set_open_override(None);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn fingerprint_post_open_nonregular_fails_closed() {
        fn open_dev_null(_physical: &Path, _display: &str) -> Result<fs::File, SecureFileError> {
            fs::File::open("/dev/null")
                .map_err(|e| SecureFileError::io(SecureFileOperation::Open, "logical/source", e))
        }
        set_open_override(Some(open_dev_null));
        let err =
            fingerprint_regular_file_nofollow(Path::new("/tmp"), "logical/source").unwrap_err();
        set_open_override(None);
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(!err.to_string().contains("/dev/null"));
        assert!(!err.to_string().contains("/tmp"));
    }
}

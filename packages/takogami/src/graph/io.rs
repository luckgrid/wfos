//! Race-safe no-follow regular-file reads for graph freshness and load.
//!
//! Thin wrappers over [`crate::contracts::secure_file`] that preserve graph-local
//! types (`GraphSourceFingerprint`) and `pub(super)` visibility for validate.

use std::path::Path;

use super::types::GraphSourceFingerprint;
use crate::contracts::secure_file::sha256_hex_regular_nofollow;

pub(super) use crate::contracts::secure_file::{SecureFileError, read_bounded_nofollow};

#[cfg(test)]
pub(super) use crate::contracts::secure_file::{
    SecureFileOperation, open_regular_nofollow, set_open_override,
};

/// Stream SHA-256 of a no-follow regular file without full-file allocation.
pub(super) fn sha256_regular_nofollow(
    physical: &Path,
    display: &str,
) -> Result<GraphSourceFingerprint, SecureFileError> {
    let digest = sha256_hex_regular_nofollow(physical, display)?;
    Ok(GraphSourceFingerprint {
        path: display.to_string(),
        algorithm: "sha256".into(),
        digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

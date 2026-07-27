//! Source fingerprint helpers (SHA-256 over raw authored bytes).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;

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

/// Fingerprint a no-follow regular file via streaming SHA-256.
///
/// Rejects missing paths, symlinks, directories, and other non-regular files.
/// `display_path` is the stable logical label stored on the fingerprint (never an absolute root).
pub fn fingerprint_regular_file_nofollow(
    path: &Path,
    display_path: &str,
) -> io::Result<SourceFingerprint> {
    use std::io::Read;
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source must not be a symlink",
        ));
    }
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source must be a regular file",
        ));
    }
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(SourceFingerprint {
        path: display_path.to_string(),
        algorithm: "sha256".to_string(),
        digest,
    })
}

fn hex_digest(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

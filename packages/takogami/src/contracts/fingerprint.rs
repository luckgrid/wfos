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

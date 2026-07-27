//! Race-safe no-follow regular-file reads for graph freshness and load.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::ControllerError;

use super::types::GraphSourceFingerprint;

/// Open a path read-only without following symlinks; require a regular file.
///
/// Unix flags: `O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK`.
pub fn open_regular_nofollow(path: &Path) -> Result<fs::File, ControllerError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| map_open_err(path, e))?;

    let meta = file.metadata().map_err(|e| {
        ControllerError::graph_contract_invalid(format!("cannot fstat {}: {e}", path.display()))
    })?;
    let ft = meta.file_type();
    if ft.is_symlink() || ft.is_dir() || !ft.is_file() {
        return Err(ControllerError::graph_contract_invalid(format!(
            "{} must be a regular non-symlink file",
            path.display()
        )));
    }
    Ok(file)
}

/// Read at most `limit` bytes (+1 sentinel) from a no-follow regular file.
pub fn read_bounded_nofollow(path: &Path, limit: u64) -> Result<Vec<u8>, ControllerError> {
    let mut file = open_regular_nofollow(path)?;
    if let Ok(opened) = file.metadata()
        && opened.len() > limit
    {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "{} exceeds {} byte limit",
            path.display(),
            limit
        )));
    }
    let mut buf = Vec::new();
    let take_limit = limit.saturating_add(1);
    let mut take = (&mut file).take(take_limit);
    take.read_to_end(&mut buf).map_err(|e| {
        ControllerError::graph_contract_invalid(format!("cannot read {}: {e}", path.display()))
    })?;
    if (buf.len() as u64) > limit {
        return Err(ControllerError::graph_limit_exceeded(format!(
            "{} exceeds {} byte limit",
            path.display(),
            limit
        )));
    }
    Ok(buf)
}

/// Stream SHA-256 of a no-follow regular file without full-file allocation.
pub fn sha256_regular_nofollow(
    path: &Path,
    display_path: &str,
) -> Result<GraphSourceFingerprint, ControllerError> {
    let mut file = open_regular_nofollow(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk).map_err(|e| {
            ControllerError::graph_contract_invalid(format!("cannot hash {}: {e}", display_path))
        })?;
        if n == 0 {
            break;
        }
        hasher.write_all(&chunk[..n]).map_err(|e| {
            ControllerError::graph_contract_invalid(format!("cannot hash {}: {e}", display_path))
        })?;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(GraphSourceFingerprint {
        path: display_path.to_string(),
        algorithm: "sha256".into(),
        digest,
    })
}

fn map_open_err(path: &Path, e: std::io::Error) -> ControllerError {
    // ELOOP / EMLINK-style: symlink followed or open refused on link.
    if e.kind() == std::io::ErrorKind::NotFound {
        return ControllerError::graph_contract_invalid(format!("missing path {}", path.display()));
    }
    // FIFO/special with O_NONBLOCK typically returns ENXIO on Linux or EAGAIN;
    // treat non-regular open failures as contract.
    ControllerError::graph_contract_invalid(format!("cannot open {}: {e}", path.display()))
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
        assert!(open_regular_nofollow(&link).is_err());
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
}

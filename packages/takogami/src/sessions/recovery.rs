//! Active-writer locking and abandoned-pending recovery.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use fs2::FileExt;

use crate::contracts::state::{ensure_state_home, set_file_mode};
use crate::contracts::types::{DiagnosticRecord, RuntimeCommandRecord};

use super::store::{CommandRecordStore, SessionStoreError, utc_now_rfc3339, validate_session_id};

/// OS-released advisory lock for one session ID.
#[derive(Debug)]
pub struct SessionLock {
    _file: File,
}

impl SessionLock {
    pub fn acquire(root: &Path, session_id: &str) -> Result<Self, SessionStoreError> {
        ensure_state_home(root)?;
        validate_session_id(session_id)?;
        let path = root.join(".locks").join(format!("{session_id}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        set_file_mode(&path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }

    pub fn try_acquire(root: &Path, session_id: &str) -> Result<Option<Self>, SessionStoreError> {
        ensure_state_home(root)?;
        validate_session_id(session_id)?;
        let path = root.join(".locks").join(format!("{session_id}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        set_file_mode(&path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(SessionStoreError::Io(err)),
        }
    }
}

// ponytail: leave lock files in place; OS releases the advisory lock when `_file` closes.

/// If a pending record's lock is free, finalize it as `abandoned`.
pub fn recover_abandoned_pending(
    store: &CommandRecordStore,
    session_id: &str,
) -> Result<Option<RuntimeCommandRecord>, SessionStoreError> {
    let record = match store.read_raw(session_id) {
        Ok(r) => r,
        Err(SessionStoreError::NotFound(_)) => return Ok(None),
        Err(SessionStoreError::Contract(msg)) => {
            return Err(SessionStoreError::Contract(msg));
        }
        Err(err) => return Err(err),
    };
    if record.execution.outcome != "pending" {
        return Ok(Some(record));
    }
    let Some(lock) = store.try_lock(session_id)? else {
        // Active writer still holds the lock.
        return Ok(Some(record));
    };
    let mut abandoned = record;
    abandoned.execution.outcome = "abandoned".into();
    abandoned.ended_at = Some(utc_now_rfc3339());
    abandoned.error = Some(DiagnosticRecord {
        code: "abandoned_pending".into(),
        message: "pending record recovered after writer lock released".into(),
    });
    abandoned.validate().map_err(SessionStoreError::Contract)?;
    store.write_final(&abandoned, &lock)?;
    Ok(Some(abandoned))
}

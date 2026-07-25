//! Active-writer locking and abandoned-pending recovery.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use fs2::FileExt;

use crate::contracts::state::{ensure_state_home, set_file_mode};
use crate::contracts::types::{DiagnosticRecord, RuntimeCommandRecord};

use super::store::{CommandRecordStore, SessionStoreError, utc_now_rfc3339, validate_session_id};

/// OS-released advisory lock for one session ID.
///
/// The bound `session_id` lets [`super::store::CommandRecordStore`] reject any write whose
/// record does not belong to the session this lock was acquired for (S6.1-04).
#[derive(Debug)]
pub struct SessionLock {
    session_id: String,
    _file: File,
}

impl SessionLock {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

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
        Ok(Self {
            session_id: session_id.to_string(),
            _file: file,
        })
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
            Ok(()) => Ok(Some(Self {
                session_id: session_id.to_string(),
                _file: file,
            })),
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
    recover_abandoned_pending_with_hook(store, session_id, None)
}

/// Same as [`recover_abandoned_pending`], with an optional deterministic hook invoked after the
/// pre-lock read and before the lock attempt. Tests use this to reproduce the
/// finalize-between-read-and-lock race without relying on timing.
pub fn recover_abandoned_pending_with_hook(
    store: &CommandRecordStore,
    session_id: &str,
    pre_lock_hook: Option<&dyn Fn()>,
) -> Result<Option<RuntimeCommandRecord>, SessionStoreError> {
    let pre_lock_record = match store.read_raw(session_id) {
        Ok(r) => r,
        Err(SessionStoreError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(err),
    };
    if pre_lock_record.execution.outcome != "pending" {
        return Ok(Some(pre_lock_record));
    }
    if let Some(hook) = pre_lock_hook {
        hook();
    }
    let Some(lock) = store.try_lock(session_id)? else {
        // Active writer still holds the lock; nothing was mutated, so the pre-lock
        // snapshot is safe to surface even though it is only advisory.
        return Ok(Some(pre_lock_record));
    };

    // The pre-lock read is advisory only: re-read under the lock we now hold before
    // deciding anything, so a concurrent finalize that landed between the pre-lock read
    // and this lock acquisition is never overwritten as abandoned (S6.1-03).
    let current = match store.read_raw(session_id) {
        Ok(r) => r,
        Err(SessionStoreError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(err),
    };
    if current.execution.outcome != "pending" {
        return Ok(Some(current));
    }
    if current.session_id != pre_lock_record.session_id
        || current.plan_digest != pre_lock_record.plan_digest
    {
        return Err(SessionStoreError::Contract(
            "pending record identity changed between pre-lock read and lock acquisition".into(),
        ));
    }

    let mut abandoned = current;
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

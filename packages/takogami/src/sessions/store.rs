//! Atomic pending/final command-record persistence.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::contracts::state::{ensure_state_home, open_new_private, set_file_mode};
use crate::contracts::types::RuntimeCommandRecord;

use super::recovery::SessionLock;

const SESSION_ID_MAX: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum SessionStoreError {
    #[error("invalid session id")]
    InvalidSessionId,
    #[error("record contract invalid: {0}")]
    Contract(String),
    #[error("session record not found: {0}")]
    NotFound(String),
    #[error("session id collision: {0}")]
    Collision(String),
    #[error("record path rejected: {0}")]
    PathRejected(String),
    #[error("state I/O: {0}")]
    Io(#[from] io::Error),
}

impl SessionStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSessionId => "invalid_session_id",
            Self::Contract(_) => "record_contract_invalid",
            Self::NotFound(_) => "session_not_found",
            Self::Collision(_) => "session_id_collision",
            Self::PathRejected(_) => "state_path_rejected",
            Self::Io(_) => "state_io",
        }
    }
}

/// Opaque session IDs: `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`.
pub fn validate_session_id(id: &str) -> Result<(), SessionStoreError> {
    if id.is_empty() || id.len() > SESSION_ID_MAX {
        return Err(SessionStoreError::InvalidSessionId);
    }
    if id == "." || id == ".." {
        return Err(SessionStoreError::InvalidSessionId);
    }
    if id.contains('/') || id.contains('\\') || id.contains('\0') {
        return Err(SessionStoreError::InvalidSessionId);
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return Err(SessionStoreError::InvalidSessionId);
    };
    if !first.is_ascii_alphanumeric() {
        return Err(SessionStoreError::InvalidSessionId);
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(SessionStoreError::InvalidSessionId);
    }
    Ok(())
}

pub fn utc_now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // ponytail: second-resolution UTC stamps; upgrade to a time crate if sub-second ordering matters.
    let days = secs / 86400;
    let tod = secs % 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    // Howard Hinnant civil_from_days (proleptic Gregorian).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[derive(Debug)]
pub struct CommandRecordStore {
    root: PathBuf,
}

impl CommandRecordStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, SessionStoreError> {
        let root = root.into();
        ensure_state_home(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn record_path(&self, session_id: &str) -> Result<PathBuf, SessionStoreError> {
        validate_session_id(session_id)?;
        Ok(self.root.join(format!("{session_id}.json")))
    }

    pub fn acquire_lock(&self, session_id: &str) -> Result<SessionLock, SessionStoreError> {
        validate_session_id(session_id)?;
        SessionLock::acquire(&self.root, session_id)
    }

    pub fn try_lock(&self, session_id: &str) -> Result<Option<SessionLock>, SessionStoreError> {
        validate_session_id(session_id)?;
        SessionLock::try_acquire(&self.root, session_id)
    }

    /// Install a pending record. Fails closed on collision with an existing unrelated ID.
    pub fn write_pending(
        &self,
        record: &RuntimeCommandRecord,
        lock: &SessionLock,
    ) -> Result<(), SessionStoreError> {
        self.validate_for_write(record)?;
        check_lock_matches_record(lock, record)?;
        let path = self.record_path(&record.session_id)?;
        if path_present(&path)? {
            return Err(SessionStoreError::Collision(record.session_id.clone()));
        }
        self.atomic_replace(&path, record)
    }

    /// Atomically replace the current document for a held lock (pending PID update or final).
    ///
    /// The lock must have been acquired for this exact session, and the currently installed
    /// record must be a legal predecessor of `record`: a live `pending` record retaining the
    /// same `plan_digest`. Once a record is terminal, it is immutable through this path (S6.1-04).
    pub fn write_final(
        &self,
        record: &RuntimeCommandRecord,
        lock: &SessionLock,
    ) -> Result<(), SessionStoreError> {
        self.validate_for_write(record)?;
        check_lock_matches_record(lock, record)?;
        let path = self.record_path(&record.session_id)?;
        let current = self
            .read_path(&path, &record.session_id)
            .map_err(|err| match err {
                SessionStoreError::NotFound(id) => SessionStoreError::Contract(format!(
                    "no installed pending record to finalize for session {id}"
                )),
                other => other,
            })?;
        check_legal_transition(&current, record)?;
        self.atomic_replace(&path, record)
    }

    /// Direct final write for Deny/Gate/planned/unavailable (no lock / no PID).
    pub fn write_terminal_unlocked(
        &self,
        record: &RuntimeCommandRecord,
    ) -> Result<(), SessionStoreError> {
        self.validate_for_write(record)?;
        if record.execution.outcome == "pending" {
            return Err(SessionStoreError::Contract(
                "unlocked write cannot install pending".into(),
            ));
        }
        let path = self.record_path(&record.session_id)?;
        if path_present(&path)? {
            return Err(SessionStoreError::Collision(record.session_id.clone()));
        }
        self.atomic_replace(&path, record)
    }

    pub fn read_raw(&self, session_id: &str) -> Result<RuntimeCommandRecord, SessionStoreError> {
        let path = self.record_path(session_id)?;
        self.read_path(&path, session_id)
    }

    pub(crate) fn read_path(
        &self,
        path: &Path,
        expected_id: &str,
    ) -> Result<RuntimeCommandRecord, SessionStoreError> {
        let meta = fs::symlink_metadata(path).map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                SessionStoreError::NotFound(expected_id.to_string())
            } else {
                SessionStoreError::Io(err)
            }
        })?;
        if meta.file_type().is_symlink() {
            return Err(SessionStoreError::PathRejected(
                "symlink records are rejected".into(),
            ));
        }
        if !meta.is_file() {
            return Err(SessionStoreError::PathRejected(
                "record path is not a regular file".into(),
            ));
        }
        let bytes = fs::read(path)?;
        let record: RuntimeCommandRecord = serde_json::from_slice(&bytes)
            .map_err(|e| SessionStoreError::Contract(e.to_string()))?;
        if record.session_id != expected_id {
            return Err(SessionStoreError::Contract(
                "filename/session_id mismatch".into(),
            ));
        }
        record.validate().map_err(SessionStoreError::Contract)?;
        Ok(record)
    }

    fn validate_for_write(&self, record: &RuntimeCommandRecord) -> Result<(), SessionStoreError> {
        validate_session_id(&record.session_id)?;
        record.validate().map_err(SessionStoreError::Contract)?;
        Ok(())
    }

    fn atomic_replace(
        &self,
        final_path: &Path,
        record: &RuntimeCommandRecord,
    ) -> Result<(), SessionStoreError> {
        let mut body =
            serde_json::to_vec(record).map_err(|e| SessionStoreError::Contract(e.to_string()))?;
        body.push(b'\n');

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = self
            .root
            .join(".tmp")
            .join(format!("{}.{nonce}.json", record.session_id));
        let write_result = (|| -> Result<(), SessionStoreError> {
            let mut file = open_new_private(&tmp)?;
            file.write_all(&body)?;
            file.sync_all()?;
            drop(file);
            set_file_mode(&tmp)?;
            fs::rename(&tmp, final_path)?;
            sync_dir(&self.root)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        write_result
    }
}

/// Seam allowing lifecycle coordination to depend on record persistence abstractly.
///
/// Production wiring always resolves to [`CommandRecordStore`]. Tests use this to inject
/// deterministic write failures without relying solely on filesystem permission bits.
pub(crate) trait RecordWriter: Send + Sync {
    fn acquire_lock(&self, session_id: &str) -> Result<SessionLock, SessionStoreError>;
    fn write_pending(
        &self,
        record: &RuntimeCommandRecord,
        lock: &SessionLock,
    ) -> Result<(), SessionStoreError>;
    fn write_final(
        &self,
        record: &RuntimeCommandRecord,
        lock: &SessionLock,
    ) -> Result<(), SessionStoreError>;
    fn write_terminal_unlocked(
        &self,
        record: &RuntimeCommandRecord,
    ) -> Result<(), SessionStoreError>;
}

impl RecordWriter for CommandRecordStore {
    fn acquire_lock(&self, session_id: &str) -> Result<SessionLock, SessionStoreError> {
        CommandRecordStore::acquire_lock(self, session_id)
    }

    fn write_pending(
        &self,
        record: &RuntimeCommandRecord,
        lock: &SessionLock,
    ) -> Result<(), SessionStoreError> {
        CommandRecordStore::write_pending(self, record, lock)
    }

    fn write_final(
        &self,
        record: &RuntimeCommandRecord,
        lock: &SessionLock,
    ) -> Result<(), SessionStoreError> {
        CommandRecordStore::write_final(self, record, lock)
    }

    fn write_terminal_unlocked(
        &self,
        record: &RuntimeCommandRecord,
    ) -> Result<(), SessionStoreError> {
        CommandRecordStore::write_terminal_unlocked(self, record)
    }
}

/// A lock only authorizes writing the record for the session it was acquired for (S6.1-04).
fn check_lock_matches_record(
    lock: &SessionLock,
    record: &RuntimeCommandRecord,
) -> Result<(), SessionStoreError> {
    if lock.session_id() != record.session_id {
        return Err(SessionStoreError::Contract(format!(
            "lock held for session `{}` cannot authorize writing session `{}`",
            lock.session_id(),
            record.session_id
        )));
    }
    Ok(())
}

/// `pending` is the only outcome a [`CommandRecordStore::write_final`] replacement may
/// originate from; once terminal, a record is immutable (S6.1-04). A byte-identical retry of
/// the already-installed terminal record is tolerated as a no-op for idempotent callers.
///
/// Addendum §5.2: a terminal (or PID) update must retain the same `plan_digest`, request
/// identity, `profile_id`, and `policy_decision` as the installed pending record.
fn check_legal_transition(
    current: &RuntimeCommandRecord,
    next: &RuntimeCommandRecord,
) -> Result<(), SessionStoreError> {
    if current.plan_digest != next.plan_digest {
        return Err(SessionStoreError::Contract(
            "final replace must retain the installed pending record's plan_digest".into(),
        ));
    }
    if current.profile_id != next.profile_id {
        return Err(SessionStoreError::Contract(
            "final replace must retain the installed pending record's profile_id".into(),
        ));
    }
    if current.policy_decision != next.policy_decision {
        return Err(SessionStoreError::Contract(
            "final replace must retain the installed pending record's policy_decision".into(),
        ));
    }
    if current.request != next.request {
        return Err(SessionStoreError::Contract(
            "final replace must retain the installed pending record's request identity".into(),
        ));
    }
    if current.execution.outcome != "pending" {
        if current == next {
            return Ok(());
        }
        return Err(SessionStoreError::Contract(format!(
            "cannot replace terminal record (outcome={}) via write_final",
            current.execution.outcome
        )));
    }
    Ok(())
}

fn path_present(path: &Path) -> Result<bool, SessionStoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(SessionStoreError::Io(err)),
    }
}

fn sync_dir(path: &Path) -> io::Result<()> {
    let dir = File::open(path)?;
    dir.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::types::{
        ExecutionRecord, OutputSummary, PolicyDecision, RECORD_KIND_COMMAND_EXECUTION,
        RequestRecord, SCHEMA_VERSION,
    };

    fn sample(outcome: &str, ended: bool) -> RuntimeCommandRecord {
        RuntimeCommandRecord {
            schema_version: SCHEMA_VERSION.into(),
            record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
            session_id: "tkg_1_2_3".into(),
            plan_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into(),
            parent_session_id: None,
            work_session_id: None,
            runtime_context: None,
            started_at: "2026-07-24T00:00:00Z".into(),
            ended_at: ended.then(|| "2026-07-24T00:00:01Z".into()),
            actor: "agent".into(),
            profile_id: "workspace-dev".into(),
            request: RequestRecord {
                command: "build".into(),
                unit_id: Some("demo".into()),
                verb: Some("build".into()),
                flags: vec![],
            },
            resolution: None,
            policy_decision: PolicyDecision::Allow {
                matched_rules: vec![],
            },
            execution: ExecutionRecord {
                started: false,
                pid: None,
                exit_code: None,
                signal: None,
                outcome: outcome.into(),
            },
            source_fingerprints: vec![],
            output_summary: OutputSummary {
                stdout_bytes: 0,
                stderr_bytes: 0,
                truncated: false,
                encoding: "utf-8".into(),
                compressor: "none".into(),
            },
            error: None,
        }
    }

    #[test]
    fn rejects_traversal_ids() {
        for id in ["../x", "a/b", "a\\b", "", ".", "..", "bad id"] {
            assert!(validate_session_id(id).is_err(), "{id}");
        }
        assert!(validate_session_id("tkg_1_2_3").is_ok());
    }

    #[test]
    fn atomic_pending_then_final() {
        let temp = tempfile::tempdir().unwrap();
        let store = CommandRecordStore::open(temp.path()).unwrap();
        let lock = store.acquire_lock("tkg_1_2_3").unwrap();
        let mut pending = sample("pending", false);
        store.write_pending(&pending, &lock).unwrap();
        pending.execution.started = true;
        pending.execution.pid = Some(42);
        store.write_final(&pending, &lock).unwrap();
        let mut final_rec = pending;
        final_rec.execution.outcome = "completed".into();
        final_rec.execution.exit_code = Some(0);
        final_rec.ended_at = Some("2026-07-24T00:00:02Z".into());
        store.write_final(&final_rec, &lock).unwrap();
        let got = store.read_raw("tkg_1_2_3").unwrap();
        assert_eq!(got.execution.outcome, "completed");
        assert_eq!(got.execution.pid, Some(42));
    }
}

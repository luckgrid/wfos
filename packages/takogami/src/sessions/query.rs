//! list / show / latest over operational command records.

use std::fs;
use std::path::Path;

use crate::contracts::types::RuntimeCommandRecord;

use super::recovery::recover_abandoned_pending;
use super::store::{CommandRecordStore, SessionStoreError, validate_session_id};

pub const DEFAULT_LIST_LIMIT: usize = 50;
pub const MAX_LIST_LIMIT: usize = 500;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub record_kind: String,
    pub session_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub profile_id: String,
    pub plan_digest: String,
    pub outcome: String,
    pub started: bool,
    pub pid: Option<u32>,
}

impl From<&RuntimeCommandRecord> for SessionSummary {
    fn from(record: &RuntimeCommandRecord) -> Self {
        Self {
            record_kind: record.record_kind.clone(),
            session_id: record.session_id.clone(),
            started_at: record.started_at.clone(),
            ended_at: record.ended_at.clone(),
            profile_id: record.profile_id.clone(),
            plan_digest: record.plan_digest.clone(),
            outcome: record.execution.outcome.clone(),
            started: record.execution.started,
            pid: record.execution.pid,
        }
    }
}

#[derive(Debug, Default)]
pub struct QueryDiagnostics {
    pub skipped: Vec<String>,
}

fn parse_limit(limit: Option<usize>) -> Result<usize, SessionStoreError> {
    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT);
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(SessionStoreError::Contract(format!(
            "list limit must be 1..={MAX_LIST_LIMIT}"
        )));
    }
    Ok(limit)
}

fn load_sorted(
    store: &CommandRecordStore,
) -> Result<(Vec<RuntimeCommandRecord>, QueryDiagnostics), SessionStoreError> {
    let mut diagnostics = QueryDiagnostics::default();
    let mut records = Vec::new();
    let root = store.root();
    if !root.exists() {
        return Ok((records, diagnostics));
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let Some(session_id) = name.strip_suffix(".json") else {
            continue;
        };
        if validate_session_id(session_id).is_err() {
            diagnostics
                .skipped
                .push(format!("{session_id}: invalid id"));
            continue;
        }
        if !is_regular_record_file(&path) {
            diagnostics.skipped.push(format!(
                "{session_id}: symlink or non-regular file rejected"
            ));
            continue;
        }
        match recover_abandoned_pending(store, session_id) {
            Ok(Some(record)) => records.push(record),
            Ok(None) => {}
            Err(SessionStoreError::Contract(msg)) => {
                diagnostics.skipped.push(format!("{session_id}: {msg}"));
            }
            Err(SessionStoreError::PathRejected(msg)) => {
                diagnostics.skipped.push(format!("{session_id}: {msg}"));
            }
            Err(err) => return Err(err),
        }
    }
    records.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.session_id.cmp(&a.session_id))
    });
    Ok((records, diagnostics))
}

pub fn list_sessions(
    store: &CommandRecordStore,
    limit: Option<usize>,
) -> Result<(Vec<SessionSummary>, QueryDiagnostics), SessionStoreError> {
    let limit = parse_limit(limit)?;
    let (records, diagnostics) = load_sorted(store)?;
    let summaries = records
        .iter()
        .take(limit)
        .map(SessionSummary::from)
        .collect();
    Ok((summaries, diagnostics))
}

pub fn show_session(
    store: &CommandRecordStore,
    session_id: &str,
) -> Result<RuntimeCommandRecord, SessionStoreError> {
    validate_session_id(session_id)?;
    match recover_abandoned_pending(store, session_id)? {
        Some(record) => Ok(record),
        None => Err(SessionStoreError::NotFound(session_id.to_string())),
    }
}

pub fn show_latest(store: &CommandRecordStore) -> Result<RuntimeCommandRecord, SessionStoreError> {
    let (records, _) = load_sorted(store)?;
    records
        .into_iter()
        .next()
        .ok_or_else(|| SessionStoreError::NotFound("latest".into()))
}

pub fn is_regular_record_file(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.is_file() && !meta.file_type().is_symlink(),
        Err(_) => false,
    }
}

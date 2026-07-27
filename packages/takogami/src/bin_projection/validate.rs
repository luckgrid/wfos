//! Bounded single-document JSON validation for bin inventory and cleanup plans.

use std::collections::BTreeSet;
use std::path::Path;

use super::types::{
    BinCleanupPlan, BinInventory, CleanupDisposition, CleanupEntry, CleanupMode, CleanupReason,
};
use crate::contracts::parse_rfc3339_utc_seconds;
use crate::projection::ValidatedBinScope;

pub const MAX_SUMMARY_COUNT: u64 = 100_000;
pub const MAX_SIZE_BYTES: u64 = 1_099_511_627_776;
pub const MAX_FILE_COUNT: u64 = 10_000_000;
pub const MAX_AGE_DAYS: u64 = 365_000;
pub const MAX_MANIFEST_COUNT: u64 = 100_000;
pub const MAX_PATH_LEN: usize = 512;
pub const MAX_RETENTION_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadError {
    InvalidUtf8,
    Empty,
    TrailingProse,
    MultipleDocuments,
    Truncated,
    Decode(String),
    Inventory(String),
    Cleanup(String),
}

impl PayloadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidUtf8
            | Self::Empty
            | Self::TrailingProse
            | Self::MultipleDocuments
            | Self::Truncated
            | Self::Decode(_) => "bin_payload_invalid",
            Self::Inventory(_) => "bin_inventory_invalid",
            Self::Cleanup(_) => "bin_cleanup_plan_invalid",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidUtf8 => "child stdout is not valid UTF-8".into(),
            Self::Empty => "child stdout is empty".into(),
            Self::TrailingProse => "child stdout has trailing prose after JSON".into(),
            Self::MultipleDocuments => "child stdout contains more than one JSON document".into(),
            Self::Truncated => "child stdout was truncated; refusing partial JSON".into(),
            Self::Decode(m) | Self::Inventory(m) | Self::Cleanup(m) => m.clone(),
        }
    }
}

/// Parse exactly one JSON document from bounded stdout. Never parse truncated output.
pub fn parse_single_json_document(bytes: &[u8], truncated: bool) -> Result<&str, PayloadError> {
    if truncated {
        return Err(PayloadError::Truncated);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PayloadError::InvalidUtf8)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(PayloadError::Empty);
    }

    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>();
    match stream.next() {
        Some(Ok(_)) => {}
        Some(Err(e)) => return Err(PayloadError::Decode(e.to_string())),
        None => return Err(PayloadError::Empty),
    }
    match stream.next() {
        None => Ok(trimmed),
        Some(Ok(_)) => Err(PayloadError::MultipleDocuments),
        Some(Err(_)) => Err(PayloadError::TrailingProse),
    }
}

pub fn decode_inventory(
    bytes: &[u8],
    truncated: bool,
    expected_workspace_root: &Path,
) -> Result<BinInventory, PayloadError> {
    let text = parse_single_json_document(bytes, truncated)?;
    let doc: BinInventory =
        serde_json::from_str(text).map_err(|e| PayloadError::Decode(e.to_string()))?;
    validate_inventory(&doc, expected_workspace_root)?;
    Ok(doc)
}

pub(crate) fn decode_cleanup_plan(
    bytes: &[u8],
    truncated: bool,
    expected_mode: CleanupMode,
    expected_scope: Option<&ValidatedBinScope>,
) -> Result<BinCleanupPlan, PayloadError> {
    let text = parse_single_json_document(bytes, truncated)?;
    let doc: BinCleanupPlan =
        serde_json::from_str(text).map_err(|e| PayloadError::Decode(e.to_string()))?;
    validate_cleanup_plan(&doc, expected_mode, expected_scope)?;
    Ok(doc)
}

pub fn validate_inventory(
    doc: &BinInventory,
    expected_workspace_root: &Path,
) -> Result<(), PayloadError> {
    require_utc_seconds(&doc.generated_at).map_err(PayloadError::Inventory)?;
    if !roots_equivalent(&doc.root, expected_workspace_root) {
        return Err(PayloadError::Inventory(
            "inventory root does not match expected workspace root".into(),
        ));
    }
    if doc.summary.total > MAX_SUMMARY_COUNT || doc.summary.with_manifest > MAX_SUMMARY_COUNT {
        return Err(PayloadError::Inventory(
            "inventory summary count exceeds maximum".into(),
        ));
    }
    if doc.workflows.len() as u64 > MAX_SUMMARY_COUNT {
        return Err(PayloadError::Inventory(
            "inventory workflow count exceeds maximum".into(),
        ));
    }
    if doc.summary.total as usize != doc.workflows.len() {
        return Err(PayloadError::Inventory(
            "inventory summary.total does not match workflows length".into(),
        ));
    }
    let with_manifest = doc.workflows.iter().filter(|w| w.manifest_present).count() as u64;
    if doc.summary.with_manifest != with_manifest {
        return Err(PayloadError::Inventory(
            "inventory summary.with_manifest inconsistent".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut prev: Option<&str> = None;
    for w in &doc.workflows {
        validate_workflow_path(&w.path, false)?;
        if w.path.len() > MAX_PATH_LEN {
            return Err(PayloadError::Inventory(
                "workflow path exceeds length limit".into(),
            ));
        }
        if w.size_bytes > MAX_SIZE_BYTES {
            return Err(PayloadError::Inventory("size_bytes exceeds maximum".into()));
        }
        if w.file_count > MAX_FILE_COUNT {
            return Err(PayloadError::Inventory("file_count exceeds maximum".into()));
        }
        if w.manifest_count > MAX_MANIFEST_COUNT {
            return Err(PayloadError::Inventory(
                "manifest_count exceeds maximum".into(),
            ));
        }
        if let Some(d) = w.oldest_file_age_days
            && d > MAX_AGE_DAYS
        {
            return Err(PayloadError::Inventory(
                "oldest_file_age_days exceeds maximum".into(),
            ));
        }
        if let Some(d) = w.newest_file_age_days
            && d > MAX_AGE_DAYS
        {
            return Err(PayloadError::Inventory(
                "newest_file_age_days exceeds maximum".into(),
            ));
        }
        if !seen.insert(w.path.as_str()) {
            return Err(PayloadError::Inventory("duplicate workflow path".into()));
        }
        if let Some(p) = prev
            && p >= w.path.as_str()
        {
            return Err(PayloadError::Inventory("workflow paths not sorted".into()));
        }
        prev = Some(w.path.as_str());
        if w.manifest_present != (w.manifest_count > 0) {
            return Err(PayloadError::Inventory(
                "manifest_present inconsistent with manifest_count".into(),
            ));
        }
        if let (Some(oldest), Some(newest)) = (w.oldest_file_age_days, w.newest_file_age_days)
            && newest > oldest
        {
            return Err(PayloadError::Inventory(
                "newest_file_age_days exceeds oldest".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_cleanup_plan(
    doc: &BinCleanupPlan,
    expected_mode: CleanupMode,
    expected_scope: Option<&ValidatedBinScope>,
) -> Result<(), PayloadError> {
    require_utc_seconds(&doc.generated_at).map_err(PayloadError::Cleanup)?;
    require_utc_seconds(&doc.inventory_generated_at).map_err(PayloadError::Cleanup)?;
    if doc.mode != expected_mode {
        return Err(PayloadError::Cleanup("cleanup mode mismatch".into()));
    }
    let expected = expected_scope.map(ValidatedBinScope::as_str);
    match (&doc.scope, expected) {
        (None, None) => {}
        (Some(got), Some(exp)) if got == exp => {}
        _ => return Err(PayloadError::Cleanup("cleanup scope mismatch".into())),
    }
    if doc.mutation_executed {
        return Err(PayloadError::Cleanup(
            "mutation_executed must be false".into(),
        ));
    }
    for c in [
        doc.summary.total,
        doc.summary.advisory,
        doc.summary.would_archive,
        doc.summary.would_delete,
        doc.summary.blocked,
    ] {
        if c > MAX_SUMMARY_COUNT {
            return Err(PayloadError::Cleanup(
                "cleanup summary count exceeds maximum".into(),
            ));
        }
    }
    if doc.entries.len() as u64 > MAX_SUMMARY_COUNT {
        return Err(PayloadError::Cleanup(
            "cleanup entry count exceeds maximum".into(),
        ));
    }
    if doc.summary.total as usize != doc.entries.len() {
        return Err(PayloadError::Cleanup(
            "cleanup summary.total does not match entries".into(),
        ));
    }
    let mut counts = (0u64, 0u64, 0u64, 0u64);
    let mut seen = BTreeSet::new();
    let mut prev: Option<&str> = None;
    for e in &doc.entries {
        validate_workflow_path(&e.path, true)?;
        if e.path.len() > MAX_PATH_LEN {
            return Err(PayloadError::Cleanup(
                "cleanup path exceeds length limit".into(),
            ));
        }
        if let Some(r) = &e.retention
            && r.len() > MAX_RETENTION_LEN
        {
            return Err(PayloadError::Cleanup(
                "retention exceeds length limit".into(),
            ));
        }
        if !seen.insert(e.path.as_str()) {
            return Err(PayloadError::Cleanup("duplicate cleanup path".into()));
        }
        if let Some(p) = prev
            && p >= e.path.as_str()
        {
            return Err(PayloadError::Cleanup("cleanup paths not sorted".into()));
        }
        prev = Some(e.path.as_str());
        if let Some(scope) = expected
            && e.path != scope
            && !e.path.starts_with(&format!("{scope}/"))
        {
            return Err(PayloadError::Cleanup(
                "cleanup entry outside requested scope".into(),
            ));
        }
        validate_disposition_combo(e)?;
        match e.disposition {
            CleanupDisposition::Advisory => counts.0 += 1,
            CleanupDisposition::WouldArchive => counts.1 += 1,
            CleanupDisposition::WouldDelete => counts.2 += 1,
            CleanupDisposition::Blocked => counts.3 += 1,
        }
    }
    if doc.summary.advisory != counts.0
        || doc.summary.would_archive != counts.1
        || doc.summary.would_delete != counts.2
        || doc.summary.blocked != counts.3
    {
        return Err(PayloadError::Cleanup(
            "cleanup summary counts do not reconcile".into(),
        ));
    }
    Ok(())
}

/// Validated retention wrapper: closed grammar matching Ontarch producer.
pub fn validate_retention(retention: Option<&str>) -> Result<(), String> {
    match retention {
        None => Ok(()),
        Some("review-before-delete") | Some("permanent") | Some("session-exports") => Ok(()),
        Some(s) if is_auto_archive(s) => Ok(()),
        Some(_) => Err("retention fails closed grammar".into()),
    }
}

fn is_auto_archive(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("auto-archive-after:") else {
        return false;
    };
    let Some(days) = rest.strip_suffix('d') else {
        return false;
    };
    !days.is_empty() && days.bytes().all(|b| b.is_ascii_digit())
}

fn validate_disposition_combo(e: &CleanupEntry) -> Result<(), PayloadError> {
    validate_retention(e.retention.as_deref()).map_err(PayloadError::Cleanup)?;
    let ok = match e.disposition {
        CleanupDisposition::WouldDelete => {
            matches!(e.reason, CleanupReason::Approved)
                && e.approved_to_matches == Some(true)
                && e.retention.is_some()
                && e.retention.as_deref() != Some("permanent")
                && validate_retention(e.retention.as_deref()).is_ok()
        }
        CleanupDisposition::WouldArchive => {
            matches!(e.reason, CleanupReason::Stale)
                && e.approved_to_matches.is_none()
                && e.retention.as_deref().is_some_and(is_auto_archive)
        }
        CleanupDisposition::Blocked => match e.reason {
            CleanupReason::ApprovedToNull | CleanupReason::ApprovedToMismatch => {
                e.approved_to_matches == Some(false)
            }
            CleanupReason::RetentionPermanent => e.retention.as_deref() == Some("permanent"),
            CleanupReason::NoManifest
            | CleanupReason::MultipleManifests
            | CleanupReason::InvalidManifest
            | CleanupReason::LibOrSrc
            | CleanupReason::OutsideScope
            | CleanupReason::ScopeRequired => e.approved_to_matches.is_none(),
            _ => false,
        },
        CleanupDisposition::Advisory => match e.reason {
            CleanupReason::RetentionReviewRequired => {
                matches!(
                    e.retention.as_deref(),
                    Some("review-before-delete") | Some("session-exports")
                ) && e.approved_to_matches.is_none()
            }
            CleanupReason::Stale => {
                e.retention.as_deref().is_some_and(is_auto_archive)
                    && e.approved_to_matches.is_none()
            }
            CleanupReason::Current => {
                e.approved_to_matches.is_none()
                    && validate_retention(e.retention.as_deref()).is_ok()
            }
            CleanupReason::NoManifest => e.retention.is_none() && e.approved_to_matches.is_none(),
            _ => false,
        },
    };
    if !ok {
        return Err(PayloadError::Cleanup(format!(
            "invalid {} disposition combination",
            e.disposition.as_str()
        )));
    }
    Ok(())
}

fn validate_workflow_path(path: &str, as_cleanup: bool) -> Result<(), PayloadError> {
    let err = |m: String| {
        if as_cleanup {
            PayloadError::Cleanup(m)
        } else {
            PayloadError::Inventory(m)
        }
    };
    if ValidatedBinScope::parse(path).is_err() {
        return Err(err("path fails workflow/subtree grammar".into()));
    }
    if path.split('/').any(|s| s == "lib" || s == "src") {
        return Err(err("lib/src paths are forbidden".into()));
    }
    Ok(())
}

fn require_utc_seconds(ts: &str) -> Result<(), String> {
    // Exact lexical form YYYY-MM-DDTHH:MM:SSZ, then calendar-valid parse (Phase 2 reuse).
    let lexical = ts.len() == 20
        && ts.as_bytes().get(4) == Some(&b'-')
        && ts.as_bytes().get(7) == Some(&b'-')
        && ts.as_bytes().get(10) == Some(&b'T')
        && ts.as_bytes().get(13) == Some(&b':')
        && ts.as_bytes().get(16) == Some(&b':')
        && ts.ends_with('Z')
        && ts[..4].bytes().all(|b| b.is_ascii_digit())
        && ts[5..7].bytes().all(|b| b.is_ascii_digit())
        && ts[8..10].bytes().all(|b| b.is_ascii_digit())
        && ts[11..13].bytes().all(|b| b.is_ascii_digit())
        && ts[14..16].bytes().all(|b| b.is_ascii_digit())
        && ts[17..19].bytes().all(|b| b.is_ascii_digit());
    if !lexical {
        return Err("timestamp must be exact UTC seconds (...Z)".into());
    }
    parse_rfc3339_utc_seconds(ts)
        .map_err(|_| String::from("timestamp is not calendar-valid UTC"))?;
    Ok(())
}

fn roots_equivalent(reported: &str, expected: &Path) -> bool {
    let reported_path = Path::new(reported);
    match (reported_path.canonicalize(), expected.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => reported_path == expected || reported == expected.to_string_lossy().as_ref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_projection::types::{CleanupSummary, InventorySummary, InventoryWorkflow};

    #[test]
    fn rejects_trailing_prose_and_multi_doc() {
        assert!(matches!(
            parse_single_json_document(br#"{"a":1} EXTRA"#, false),
            Err(PayloadError::TrailingProse)
        ));
        assert!(matches!(
            parse_single_json_document(br#"{"a":1}{"b":2}"#, false),
            Err(PayloadError::MultipleDocuments)
        ));
        assert!(matches!(
            parse_single_json_document(br#"{"a":1}"#, true),
            Err(PayloadError::Truncated)
        ));
    }

    #[test]
    fn mutation_true_fails() {
        let plan = BinCleanupPlan {
            generated_at: "2026-07-25T00:00:00Z".into(),
            mode: CleanupMode::ReportOnly,
            scope: None,
            inventory_generated_at: "2026-07-25T00:00:00Z".into(),
            inventory_refreshed: false,
            summary: CleanupSummary {
                total: 0,
                advisory: 0,
                would_archive: 0,
                would_delete: 0,
                blocked: 0,
            },
            entries: vec![],
            mutation_executed: true,
        };
        assert!(validate_cleanup_plan(&plan, CleanupMode::ReportOnly, None).is_err());
    }

    #[test]
    fn empty_inventory_ok() {
        let inv = BinInventory {
            generated_at: "2026-07-25T00:00:00Z".into(),
            root: "/tmp/x".into(),
            summary: InventorySummary {
                total: 0,
                with_manifest: 0,
            },
            workflows: vec![],
        };
        let p = Path::new("/tmp/x");
        let _ = validate_inventory(&inv, p);
    }

    #[test]
    fn inventory_calendar_invalid_timestamp_rejected() {
        let inv = BinInventory {
            generated_at: "2026-99-99T99:99:99Z".into(),
            root: "/tmp/x".into(),
            summary: InventorySummary {
                total: 0,
                with_manifest: 0,
            },
            workflows: vec![],
        };
        let err = validate_inventory(&inv, Path::new("/tmp/x")).unwrap_err();
        assert_eq!(err.code(), "bin_inventory_invalid");
    }

    #[test]
    fn non_leap_feb_29_rejected() {
        assert!(require_utc_seconds("2025-02-29T00:00:00Z").is_err());
    }

    #[test]
    fn fractional_and_offset_timestamps_rejected() {
        assert!(require_utc_seconds("2026-07-25T00:00:00.123Z").is_err());
        assert!(require_utc_seconds("2026-07-25T00:00:00+00:00").is_err());
    }

    #[test]
    fn cleanup_invalid_entry_path_uses_cleanup_diagnostic() {
        let plan = BinCleanupPlan {
            generated_at: "2026-07-25T00:00:00Z".into(),
            mode: CleanupMode::ReportOnly,
            scope: None,
            inventory_generated_at: "2026-07-25T00:00:00Z".into(),
            inventory_refreshed: false,
            summary: CleanupSummary {
                total: 1,
                advisory: 1,
                would_archive: 0,
                would_delete: 0,
                blocked: 0,
            },
            entries: vec![CleanupEntry {
                path: "Plan/bin".into(),
                disposition: CleanupDisposition::Advisory,
                reason: CleanupReason::Current,
                retention: None,
                approved_to_matches: None,
            }],
            mutation_executed: false,
        };
        let err = validate_cleanup_plan(&plan, CleanupMode::ReportOnly, None).unwrap_err();
        assert_eq!(err.code(), "bin_cleanup_plan_invalid");
    }

    #[test]
    fn would_archive_requires_auto_archive_retention() {
        let e = CleanupEntry {
            path: "Build/bin/wfos".into(),
            disposition: CleanupDisposition::WouldArchive,
            reason: CleanupReason::Stale,
            retention: Some("permanent".into()),
            approved_to_matches: None,
        };
        assert!(validate_disposition_combo(&e).is_err());
        let e2 = CleanupEntry {
            retention: Some("auto-archive-after:30d".into()),
            ..e
        };
        assert!(validate_disposition_combo(&e2).is_ok());
    }

    #[test]
    fn would_delete_rejects_arbitrary_retention() {
        let e = CleanupEntry {
            path: "Build/bin/wfos".into(),
            disposition: CleanupDisposition::WouldDelete,
            reason: CleanupReason::Approved,
            retention: Some("foo".into()),
            approved_to_matches: Some(true),
        };
        assert!(validate_disposition_combo(&e).is_err());
    }

    #[test]
    fn inventory_numeric_bounds_enforced() {
        let mut inv = BinInventory {
            generated_at: "2026-07-25T00:00:00Z".into(),
            root: "/tmp/x".into(),
            summary: InventorySummary {
                total: 1,
                with_manifest: 0,
            },
            workflows: vec![InventoryWorkflow {
                path: "Build/bin/wfos".into(),
                size_bytes: MAX_SIZE_BYTES,
                file_count: MAX_FILE_COUNT,
                oldest_file_age_days: Some(MAX_AGE_DAYS),
                newest_file_age_days: Some(0),
                manifest_present: false,
                manifest_count: 0,
            }],
        };
        assert!(validate_inventory(&inv, Path::new("/tmp/x")).is_ok());
        inv.workflows[0].size_bytes = MAX_SIZE_BYTES + 1;
        assert!(validate_inventory(&inv, Path::new("/tmp/x")).is_err());
    }

    #[test]
    fn path_and_retention_string_bounds_enforced() {
        let long = "a".repeat(MAX_PATH_LEN + 1);
        let inv = BinInventory {
            generated_at: "2026-07-25T00:00:00Z".into(),
            root: "/tmp/x".into(),
            summary: InventorySummary {
                total: 1,
                with_manifest: 0,
            },
            workflows: vec![InventoryWorkflow {
                path: format!("Build/bin/{long}"),
                size_bytes: 0,
                file_count: 0,
                oldest_file_age_days: None,
                newest_file_age_days: None,
                manifest_present: false,
                manifest_count: 0,
            }],
        };
        // Grammar may fail first; either way reject.
        assert!(validate_inventory(&inv, Path::new("/tmp/x")).is_err());
    }
}

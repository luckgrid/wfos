//! Bounded single-document JSON validation for bin inventory and cleanup plans.

use std::collections::BTreeSet;
use std::path::Path;

use super::types::{BinCleanupPlan, BinInventory, CleanupDisposition, CleanupMode, CleanupReason};
use crate::projection::ValidatedBinScope;

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
        validate_workflow_path(&w.path)?;
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
    if doc.summary.total as usize != doc.entries.len() {
        return Err(PayloadError::Cleanup(
            "cleanup summary.total does not match entries".into(),
        ));
    }
    let mut counts = (0u64, 0u64, 0u64, 0u64);
    let mut seen = BTreeSet::new();
    let mut prev: Option<&str> = None;
    for e in &doc.entries {
        validate_workflow_path(&e.path)?;
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

fn validate_disposition_combo(e: &super::types::CleanupEntry) -> Result<(), PayloadError> {
    match e.disposition {
        CleanupDisposition::WouldDelete => {
            if !matches!(e.reason, CleanupReason::Approved)
                || e.approved_to_matches != Some(true)
                || e.retention.as_deref() == Some("permanent")
                || e.retention.is_none()
            {
                return Err(PayloadError::Cleanup(
                    "invalid would_delete disposition combination".into(),
                ));
            }
        }
        CleanupDisposition::WouldArchive => {
            if !matches!(e.reason, CleanupReason::Stale) || e.approved_to_matches.is_some() {
                return Err(PayloadError::Cleanup(
                    "invalid would_archive disposition combination".into(),
                ));
            }
        }
        CleanupDisposition::Advisory | CleanupDisposition::Blocked => {}
    }
    Ok(())
}

fn validate_workflow_path(path: &str) -> Result<(), PayloadError> {
    if ValidatedBinScope::parse(path).is_err() {
        return Err(PayloadError::Inventory(
            "path fails workflow/subtree grammar".into(),
        ));
    }
    if path.split('/').any(|s| s == "lib" || s == "src") {
        return Err(PayloadError::Inventory(
            "lib/src paths are forbidden".into(),
        ));
    }
    Ok(())
}

fn require_utc_seconds(ts: &str) -> Result<(), String> {
    // YYYY-MM-DDTHH:MM:SSZ
    let b = ts.as_bytes();
    if b.len() != 20 || b[19] != b'Z' || b[10] != b'T' {
        return Err("timestamp must be exact UTC seconds (...Z)".into());
    }
    let digit = |i: usize| b[i].is_ascii_digit();
    let ok = digit(0)
        && digit(1)
        && digit(2)
        && digit(3)
        && b[4] == b'-'
        && digit(5)
        && digit(6)
        && b[7] == b'-'
        && digit(8)
        && digit(9)
        && digit(11)
        && digit(12)
        && b[13] == b':'
        && digit(14)
        && digit(15)
        && b[16] == b':'
        && digit(17)
        && digit(18);
    if !ok {
        return Err("timestamp must be exact UTC seconds (...Z)".into());
    }
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
    use crate::bin_projection::types::{CleanupSummary, InventorySummary};

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
}

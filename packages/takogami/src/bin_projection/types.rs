//! Closed wire types for Ontarch bin inventory and cleanup-plan payloads.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BinInventory {
    pub generated_at: String,
    pub root: String,
    pub summary: InventorySummary,
    pub workflows: Vec<InventoryWorkflow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InventorySummary {
    pub total: u64,
    pub with_manifest: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InventoryWorkflow {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub oldest_file_age_days: Option<u64>,
    pub newest_file_age_days: Option<u64>,
    pub manifest_present: bool,
    pub manifest_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BinCleanupPlan {
    pub generated_at: String,
    pub mode: CleanupMode,
    pub scope: Option<String>,
    pub inventory_generated_at: String,
    pub inventory_refreshed: bool,
    pub summary: CleanupSummary,
    pub entries: Vec<CleanupEntry>,
    pub mutation_executed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupMode {
    ReportOnly,
    DryRun,
    Archive,
    DeleteApproved,
}

impl CleanupMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReportOnly => "report-only",
            Self::DryRun => "dry-run",
            Self::Archive => "archive",
            Self::DeleteApproved => "delete-approved",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CleanupSummary {
    pub total: u64,
    pub advisory: u64,
    pub would_archive: u64,
    pub would_delete: u64,
    pub blocked: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CleanupEntry {
    pub path: String,
    pub disposition: CleanupDisposition,
    pub reason: CleanupReason,
    pub retention: Option<String>,
    pub approved_to_matches: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupDisposition {
    Advisory,
    WouldArchive,
    WouldDelete,
    Blocked,
}

impl CleanupDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Advisory => "advisory",
            Self::WouldArchive => "would_archive",
            Self::WouldDelete => "would_delete",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupReason {
    Approved,
    ApprovedToMismatch,
    ApprovedToNull,
    Current,
    InvalidManifest,
    LibOrSrc,
    MultipleManifests,
    NoManifest,
    OutsideScope,
    RetentionPermanent,
    RetentionReviewRequired,
    ScopeRequired,
    Stale,
}

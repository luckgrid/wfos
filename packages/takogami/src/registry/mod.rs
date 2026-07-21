//! Read-only Ontarch registry access, freshness, and query helpers (E09.S3).

mod access;
mod adapters;
mod query;
mod scan;
mod types;

pub use access::{RegistryAccess, RegistryPaths, resolve_registry_paths};
pub use adapters::{ExternalAdapters, ProcessAdapters, RefreshKind};
pub use query::{UnitFilters, filter_tools, filter_units, find_unit, parse_filters};
pub use scan::{ScanDiscovery, discover_from_scan};
pub use types::{
    Freshness, ProvisionalUnit, RegistryFileKind, ScanDocument, ToolRecord, ToolsDocument,
    UnitRecord, UnitsDocument, WorkspaceScanEntry,
};

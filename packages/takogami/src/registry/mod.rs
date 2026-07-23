//! Read-only Ontarch registry access, freshness, and query helpers.

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
    AuthoredNative, AuthoredPaths, AuthoredUnitDescriptor, EntrypointDefinition, Freshness,
    PoliciesDocument, PolicyRecord, ProfileRecord, ProfilesDocument, ProvisionalUnit,
    RegistryFileKind, ScanDocument, StructuredEntrypoint, ToolRecord, ToolsDocument,
    UnitDefinition, UnitRecord, UnitsDocument, WorkspaceScanEntry,
};

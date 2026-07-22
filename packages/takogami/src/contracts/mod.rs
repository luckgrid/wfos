//! Versioned public machine contracts for the runtime controller.
//!
//! Schema version mismatches are typed contract errors — never silently coerced.
//! Environment values and secret material must never appear in these records.

pub mod entrypoint;
pub mod fingerprint;
pub mod state;
pub mod types;

pub use entrypoint::{LegacyEntrypoint, LegacyParseError, parse_legacy_entrypoint};
pub use fingerprint::{SourceFingerprint, fingerprint_bytes, fingerprint_file};
pub use state::{StateHomeInputs, resolve_session_state_home};
pub use types::{
    ChildOutput, CommandEnvelope, DiagnosticRecord, EnvelopeMetrics, ExecutionClass,
    ExecutionRecord, OutputSummary, PolicyDecision, RECORD_KIND_COMMAND_EXECUTION,
    RegistryGeneration, RequestRecord, ResolvedCommand, RuntimeCommandRecord, RuntimeContext,
    SCHEMA_VERSION,
};

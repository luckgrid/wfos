//! Operational command-execution session records (not build or work sessions).

mod query;
mod recovery;
mod runtime_context;
mod store;

pub use query::{
    DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT, QueryDiagnostics, SessionSummary, list_sessions,
    show_latest, show_latest_with_diagnostics, show_session,
};
pub use recovery::{SessionLock, recover_abandoned_pending, recover_abandoned_pending_with_hook};
pub use runtime_context::{RuntimeContextEnv, collect_runtime_context};
pub(crate) use store::RecordWriter;
pub use store::{CommandRecordStore, SessionStoreError, utc_now_rfc3339, validate_session_id};

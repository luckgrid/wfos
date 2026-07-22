//! Lifecycle request and correlation IDs.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleVerb {
    Dev,
    Build,
    Check,
}

impl LifecycleVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Build => "build",
            Self::Check => "check",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolutionRequest {
    pub session_id: String,
    pub unit_id: String,
    pub verb: LifecycleVerb,
    pub explicit_profile: Option<String>,
    pub explain: bool,
    pub execute_requested: bool,
}

pub trait CorrelationIdGenerator {
    fn next_id(&mut self) -> String;
}

#[derive(Debug, Default)]
pub struct DefaultIdGenerator {
    counter: AtomicU64,
}

impl CorrelationIdGenerator for DefaultIdGenerator {
    fn next_id(&mut self) -> String {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let pid = std::process::id();
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("tkg_{ms}_{pid}_{n}")
    }
}

#[derive(Debug, Clone)]
pub struct FixedIdGenerator {
    pub id: String,
}

impl CorrelationIdGenerator for FixedIdGenerator {
    fn next_id(&mut self) -> String {
        self.id.clone()
    }
}

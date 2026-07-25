//! Native process execution for evaluator-authorized plans only.

mod environment;
mod process;
mod signals;
mod streams;

pub use environment::{EnvError, EnvSnapshot, snapshot_env};
pub use process::TokioExecutor;
pub use signals::{NullSignalSource, ProcessGroupGuard, SignalSource, UnixSignalSource};
pub use streams::{
    DEFAULT_CAPTURE_LIMIT, StreamCapture, StreamDest, StreamEncoding, capture_pipe, classify_bytes,
    stream_or_buffer,
};

use std::cell::Cell;

use async_trait::async_trait;

use crate::contracts::types::DiagnosticRecord;
use crate::policy::AuthorizedExecutionPlan;

/// Per-stream capture bound (JSON and RTK-eligible human buffering).
pub const DEFAULT_LIMIT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    pub capture_limit: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            capture_limit: DEFAULT_LIMIT_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExecutionMode {
    Json,
    Human {
        rtk_eligible: bool,
        profile_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    pub mode: ExecutionMode,
    pub limits: ExecutionLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSummary {
    pub text: Option<String>,
    pub encoding: String,
    pub truncated: bool,
    pub total_bytes: u64,
    pub captured_bytes: u64,
}

impl StreamSummary {
    pub fn empty() -> Self {
        Self {
            text: None,
            encoding: "utf-8".into(),
            truncated: false,
            total_bytes: 0,
            captured_bytes: 0,
        }
    }

    pub fn from_capture(capture: &StreamCapture) -> Self {
        let text = match capture.encoding {
            StreamEncoding::Binary => None,
            StreamEncoding::Utf8 | StreamEncoding::LossyUtf8 => {
                Some(String::from_utf8_lossy(&capture.bytes).into_owned())
            }
        };
        Self {
            text,
            encoding: capture.encoding.as_str().into(),
            truncated: capture.truncated,
            total_bytes: capture.total_bytes,
            captured_bytes: capture.bytes.len() as u64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub spawned: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<u8>,
    pub signal: Option<String>,
    pub outcome: String,
    pub stdout: StreamSummary,
    pub stderr: StreamSummary,
    pub diagnostics: Vec<DiagnosticRecord>,
    pub compressor: String,
    pub gain: Option<f64>,
    pub emitted_output_bytes: u64,
}

impl ExecutionReport {
    pub fn idle(outcome: impl Into<String>) -> Self {
        Self {
            spawned: false,
            pid: None,
            exit_code: None,
            signal: None,
            outcome: outcome.into(),
            stdout: StreamSummary::empty(),
            stderr: StreamSummary::empty(),
            diagnostics: vec![],
            compressor: "none".into(),
            gain: None,
            emitted_output_bytes: 0,
        }
    }
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn execute(
        &self,
        plan: &AuthorizedExecutionPlan,
        options: &ExecutionOptions,
    ) -> ExecutionReport;
}

/// Production stand-in that never starts a child (kept for plan-only / test injection).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableExecutor;

#[async_trait]
impl Executor for UnavailableExecutor {
    async fn execute(
        &self,
        _plan: &AuthorizedExecutionPlan,
        _options: &ExecutionOptions,
    ) -> ExecutionReport {
        ExecutionReport::idle("execution_unavailable")
    }
}

/// Test spy that counts reachability without spawning.
#[derive(Debug, Default)]
pub struct SpyExecutor {
    pub calls: Cell<u32>,
}

// ponytail: Cell is single-threaded; Sync is required for `&dyn Executor` across `.await`.
// Tests must not share one SpyExecutor across threads. Prefer AtomicU32 if that ceiling is hit.
unsafe impl Sync for SpyExecutor {}

#[async_trait]
impl Executor for SpyExecutor {
    async fn execute(
        &self,
        _plan: &AuthorizedExecutionPlan,
        _options: &ExecutionOptions,
    ) -> ExecutionReport {
        self.calls.set(self.calls.get().saturating_add(1));
        ExecutionReport::idle("spy_reached")
    }
}

impl SpyExecutor {
    pub fn reached(&self) -> bool {
        self.calls.get() > 0
    }
}

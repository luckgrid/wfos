//! Typed wire records for command envelopes, policy decisions, and command execution records.

use serde::{Deserialize, Serialize};

use super::fingerprint::SourceFingerprint;

/// Wire schema version for all Takogami machine contracts.
pub const SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticRecord {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryGeneration {
    pub generated_at: String,
    pub source_fingerprints: Vec<SourceFingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub schema_version: String,
    pub session_id: String,
    pub unit_id: String,
    pub verb: String,
    pub descriptor_path: String,
    pub descriptor_fingerprint: String,
    pub native_manifests: Vec<String>,
    pub backend: String,
    pub adapter: String,
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: String,
    /// Environment key names only — never values.
    pub env_keys: Vec<String>,
    pub profile_id: String,
    pub policy_ids: Vec<String>,
    pub registry_generation: RegistryGeneration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow {
        matched_rules: Vec<String>,
    },
    Gate {
        policy_id: String,
        rule_id: String,
        reason: String,
        required_approval: String,
    },
    Deny {
        policy_id: String,
        rule_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    pub truncated: bool,
    pub encoding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvelopeMetrics {
    pub registry_cache: String,
    pub output_bytes: u64,
    pub compressor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandEnvelope<T = serde_json::Value> {
    pub schema_version: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub status: String,
    pub exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<serde_json::Value>,
    pub diagnostics: Vec<DiagnosticRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child: Option<ChildOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<EnvelopeMetrics>,
}

impl CommandEnvelope<serde_json::Value> {
    pub fn error(command: &str, exit_code: u8, code: &str, message: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            command: command.to_string(),
            session_id: None,
            status: "error".to_string(),
            exit_code,
            data: None,
            explanation: None,
            diagnostics: vec![DiagnosticRecord {
                code: code.to_string(),
                message: message.to_string(),
            }],
            child: None,
            metrics: None,
        }
    }

    pub fn ok(command: &str, data: Option<serde_json::Value>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            command: command.to_string(),
            session_id: None,
            status: "ok".to_string(),
            exit_code: 0,
            data,
            explanation: None,
            diagnostics: vec![],
            child: None,
            metrics: None,
        }
    }
}

/// Reject envelopes whose schema_version does not match the supported contract.
pub fn require_schema_version(version: &str) -> Result<(), String> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!(
            "schema_version mismatch: got {version}, expected {SCHEMA_VERSION}"
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestRecord {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub started: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputSummary {
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub truncated: bool,
    pub encoding: String,
    pub compressor: String,
}

/// Provider-neutral link to a terminal runtime (Herdr/tmux/direct). Opaque IDs only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeContext {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

/// Operational command execution audit record (not a composed work session).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeCommandRecord {
    pub schema_version: String,
    pub record_kind: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_context: Option<RuntimeContext>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub actor: String,
    pub profile_id: String,
    pub request: RequestRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ResolvedCommand>,
    pub policy_decision: PolicyDecision,
    pub execution: ExecutionRecord,
    pub source_fingerprints: Vec<SourceFingerprint>,
    pub output_summary: OutputSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DiagnosticRecord>,
}

/// Const value for [`RuntimeCommandRecord::record_kind`].
pub const RECORD_KIND_COMMAND_EXECUTION: &str = "command_execution";

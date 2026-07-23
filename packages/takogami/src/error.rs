use crate::contracts::PolicyDecision;
use crate::exit_codes::{
    CONTRACT, INTERNAL, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, RESOLUTION, USAGE,
};
use crate::policy::{PolicyContractError, PolicyEvaluationExplanation};
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct PolicyContractDetails {
    pub code: String,
    pub message: String,
    pub session_id: String,
    pub plan_digest: String,
    pub policy_id: Option<String>,
    pub field: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PolicyOutcomeDetails {
    pub reason: String,
    pub session_id: String,
    pub plan_digest: String,
    pub decision: PolicyDecision,
    pub explanation: PolicyEvaluationExplanation,
}

#[derive(Debug, Clone)]
pub struct ExecutionDeferredDetails {
    pub session_id: String,
    pub plan_digest: Option<String>,
    pub policy_decision: Option<PolicyDecision>,
    pub policy_explanation: Option<PolicyEvaluationExplanation>,
    pub execution_requested: bool,
}

#[derive(Debug, Error, Diagnostic)]
pub enum ControllerError {
    #[error("invalid usage: {message}")]
    #[diagnostic(code(takogami::usage))]
    Usage {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("command not implemented: {command}")]
    #[diagnostic(code(takogami::not_implemented))]
    NotImplemented { command: String },

    #[error("contract error: {message}")]
    #[diagnostic(code(takogami::contract))]
    Contract { message: String },

    #[error("not found: {message}")]
    #[diagnostic(code(takogami::not_found))]
    NotFound { message: String },

    #[error("ambiguous: {message}")]
    #[diagnostic(code(takogami::ambiguous))]
    Ambiguous { message: String },

    #[error("invalid registry: {message}")]
    #[diagnostic(code(takogami::invalid_registry))]
    InvalidRegistry { message: String },

    #[error("invalid filter: {message}")]
    #[diagnostic(code(takogami::invalid_filter))]
    InvalidFilter { message: String },

    #[error("unavailable source: {message}")]
    #[diagnostic(code(takogami::unavailable_source))]
    UnavailableSource { message: String },

    #[error("resolution failed ({code}): {message}")]
    #[diagnostic(code(takogami::resolution))]
    Resolution {
        code: String,
        message: String,
        session_id: Option<String>,
        explanation_partial: Option<Box<serde_json::Value>>,
    },

    #[error("policy contract invalid ({code}): {message}")]
    #[diagnostic(code(takogami::policy_contract))]
    PolicyContract {
        code: String,
        message: String,
        details: Box<PolicyContractDetails>,
    },

    #[error("policy deny: {reason}")]
    #[diagnostic(code(takogami::policy_deny))]
    PolicyDeny {
        reason: String,
        details: Box<PolicyOutcomeDetails>,
    },

    #[error("policy gate: {reason}")]
    #[diagnostic(code(takogami::policy_gate))]
    PolicyGate {
        reason: String,
        details: Box<PolicyOutcomeDetails>,
    },

    #[error("execution unavailable in S5 (plan-only): session={session_id}")]
    #[diagnostic(code(takogami::execution_unavailable))]
    ExecutionUnavailable {
        session_id: String,
        details: Box<ExecutionDeferredDetails>,
    },

    #[error("execution class unavailable: {message}")]
    #[diagnostic(code(takogami::execution_class_unavailable))]
    ExecutionClassUnavailable {
        message: String,
        details: Box<ExecutionDeferredDetails>,
    },

    #[error("internal error: {message}")]
    #[diagnostic(code(takogami::internal))]
    Internal { message: String },
}

impl ControllerError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
            source: None,
        }
    }

    pub fn usage_from(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Usage {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }

    pub fn not_implemented(command: impl Into<String>) -> Self {
        Self::NotImplemented {
            command: command.into(),
        }
    }

    pub fn contract(message: impl Into<String>) -> Self {
        Self::Contract {
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn ambiguous(message: impl Into<String>) -> Self {
        Self::Ambiguous {
            message: message.into(),
        }
    }

    pub fn invalid_registry(message: impl Into<String>) -> Self {
        Self::InvalidRegistry {
            message: message.into(),
        }
    }

    pub fn invalid_filter(message: impl Into<String>) -> Self {
        Self::InvalidFilter {
            message: message.into(),
        }
    }

    pub fn unavailable_source(message: impl Into<String>) -> Self {
        Self::UnavailableSource {
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn from_policy_contract(err: PolicyContractError) -> Self {
        let code = err.kind.code().to_string();
        Self::PolicyContract {
            code: code.clone(),
            message: err.message.clone(),
            details: Box::new(PolicyContractDetails {
                code,
                message: err.message,
                session_id: err.session_id,
                plan_digest: err.plan_digest,
                policy_id: err.policy_id,
                field: err.field,
            }),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Resolution { session_id, .. } => session_id.as_deref(),
            Self::PolicyContract { details, .. } => Some(details.session_id.as_str()),
            Self::PolicyDeny { details, .. } | Self::PolicyGate { details, .. } => {
                Some(details.session_id.as_str())
            }
            Self::ExecutionUnavailable { session_id, .. } => Some(session_id.as_str()),
            Self::ExecutionClassUnavailable { details, .. } => Some(details.session_id.as_str()),
            _ => None,
        }
    }

    pub fn plan_digest(&self) -> Option<&str> {
        match self {
            Self::PolicyContract { details, .. } => Some(details.plan_digest.as_str()),
            Self::PolicyDeny { details, .. } | Self::PolicyGate { details, .. } => {
                Some(details.plan_digest.as_str())
            }
            Self::ExecutionUnavailable { details, .. }
            | Self::ExecutionClassUnavailable { details, .. } => details.plan_digest.as_deref(),
            _ => None,
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage { .. } => USAGE,
            Self::NotImplemented { .. }
            | Self::ExecutionUnavailable { .. }
            | Self::ExecutionClassUnavailable { .. } => NOT_IMPLEMENTED,
            Self::Contract { .. } | Self::InvalidRegistry { .. } | Self::PolicyContract { .. } => {
                CONTRACT
            }
            Self::NotFound { .. } | Self::Ambiguous { .. } | Self::InvalidFilter { .. } => USAGE,
            Self::Resolution { .. } => RESOLUTION,
            Self::PolicyDeny { .. } => POLICY_DENY,
            Self::PolicyGate { .. } => POLICY_GATE,
            Self::UnavailableSource { .. } | Self::Internal { .. } => INTERNAL,
        }
    }

    pub fn diagnostic_code(&self) -> &str {
        match self {
            Self::Usage { .. } => "usage",
            Self::NotImplemented { .. } => "not_implemented",
            Self::Contract { .. } => "contract",
            Self::NotFound { .. } => "not_found",
            Self::Ambiguous { .. } => "ambiguous",
            Self::InvalidRegistry { .. } => "invalid_registry",
            Self::InvalidFilter { .. } => "invalid_filter",
            Self::UnavailableSource { .. } => "unavailable_source",
            Self::Resolution { code, .. } => code.as_str(),
            Self::PolicyContract { code, .. } => code.as_str(),
            Self::PolicyDeny { .. } => "policy_deny",
            Self::PolicyGate { .. } => "policy_gate",
            Self::ExecutionUnavailable { .. } => "execution_unavailable",
            Self::ExecutionClassUnavailable { .. } => "execution_class_unavailable",
            Self::Internal { .. } => "internal",
        }
    }
}

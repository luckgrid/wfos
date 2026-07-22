use crate::exit_codes::{CONTRACT, INTERNAL, NOT_IMPLEMENTED, RESOLUTION, USAGE};
use miette::Diagnostic;
use thiserror::Error;

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
        explanation_partial: Option<serde_json::Value>,
    },

    #[error("execution unavailable in S4 (plan-only): session={session_id}")]
    #[diagnostic(code(takogami::execution_unavailable))]
    ExecutionUnavailable {
        session_id: String,
        plan_digest: Option<String>,
    },

    #[error("execution class unavailable: {message}")]
    #[diagnostic(code(takogami::execution_class_unavailable))]
    ExecutionClassUnavailable {
        message: String,
        session_id: String,
        plan_digest: Option<String>,
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

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Resolution { session_id, .. } => session_id.as_deref(),
            Self::ExecutionUnavailable { session_id, .. }
            | Self::ExecutionClassUnavailable { session_id, .. } => Some(session_id.as_str()),
            _ => None,
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage { .. } => USAGE,
            Self::NotImplemented { .. }
            | Self::ExecutionUnavailable { .. }
            | Self::ExecutionClassUnavailable { .. } => NOT_IMPLEMENTED,
            Self::Contract { .. } | Self::InvalidRegistry { .. } => CONTRACT,
            Self::NotFound { .. } | Self::Ambiguous { .. } | Self::InvalidFilter { .. } => USAGE,
            Self::Resolution { .. } => RESOLUTION,
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
            Self::ExecutionUnavailable { .. } => "execution_unavailable",
            Self::ExecutionClassUnavailable { .. } => "execution_class_unavailable",
            Self::Internal { .. } => "internal",
        }
    }
}

use crate::exit_codes::{CONTRACT, INTERNAL, NOT_IMPLEMENTED, USAGE};
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

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage { .. } => USAGE,
            Self::NotImplemented { .. } => NOT_IMPLEMENTED,
            Self::Contract { .. } => CONTRACT,
            Self::Internal { .. } => INTERNAL,
        }
    }

    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Usage { .. } => "usage",
            Self::NotImplemented { .. } => "not_implemented",
            Self::Contract { .. } => "contract",
            Self::Internal { .. } => "internal",
        }
    }
}

//! Entrypoint normalization for structured and legacy forms.

use crate::contracts::{DiagnosticRecord, ExecutionClass, parse_legacy_entrypoint};
use crate::registry::{EntrypointDefinition, StructuredEntrypoint};

use super::resolver::ResolutionCode;

#[derive(Debug, Clone)]
pub struct NormalizedEntrypoint {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env_keys: Vec<String>,
    pub backend: Option<String>,
    pub adapter: Option<String>,
    pub source_manifests: Vec<String>,
    pub required_policies: Vec<String>,
    pub execution_class: ExecutionClass,
    pub runtime_provider: Option<String>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

pub fn normalize_entrypoint(
    def: &EntrypointDefinition,
) -> Result<NormalizedEntrypoint, ResolutionCode> {
    match def {
        EntrypointDefinition::Structured(s) => normalize_structured(s),
        EntrypointDefinition::Legacy(raw) => normalize_legacy(raw),
    }
}

fn normalize_structured(s: &StructuredEntrypoint) -> Result<NormalizedEntrypoint, ResolutionCode> {
    validate_token(&s.program)?;
    for a in &s.args {
        validate_token(a)?;
    }
    for k in &s.env_keys {
        validate_env_key(k)?;
    }
    validate_execution_class(s.execution_class, s.runtime_provider.as_deref())?;
    Ok(NormalizedEntrypoint {
        program: s.program.clone(),
        args: s.args.clone(),
        cwd: s.cwd.clone(),
        env_keys: s.env_keys.clone(),
        backend: s.backend.clone(),
        adapter: s.adapter.clone(),
        source_manifests: s.source_manifests.clone(),
        required_policies: s.required_policies.clone(),
        execution_class: s.execution_class,
        runtime_provider: s.runtime_provider.clone(),
        diagnostics: Vec::new(),
    })
}

fn normalize_legacy(raw: &str) -> Result<NormalizedEntrypoint, ResolutionCode> {
    let parsed = parse_legacy_entrypoint(raw)
        .map_err(|e| ResolutionCode::UnsafeLegacyEntrypoint { message: e.message })?;
    Ok(NormalizedEntrypoint {
        program: parsed.program,
        args: parsed.args,
        cwd: None,
        env_keys: Vec::new(),
        backend: None,
        adapter: None,
        source_manifests: Vec::new(),
        required_policies: Vec::new(),
        execution_class: ExecutionClass::Direct,
        runtime_provider: None,
        diagnostics: vec![parsed.deprecation],
    })
}

fn validate_execution_class(
    class: ExecutionClass,
    provider: Option<&str>,
) -> Result<(), ResolutionCode> {
    match class {
        ExecutionClass::Direct => {
            if provider.is_some() {
                return Err(ResolutionCode::InvalidDescriptor {
                    message: "execution_class=direct requires runtime_provider null/absent".into(),
                });
            }
            Ok(())
        }
        ExecutionClass::InteractiveSession => {
            if provider.map(str::trim).filter(|s| !s.is_empty()).is_none() {
                return Err(ResolutionCode::InvalidDescriptor {
                    message:
                        "execution_class=interactive_session requires non-empty runtime_provider"
                            .into(),
                });
            }
            Ok(())
        }
    }
}

fn validate_token(s: &str) -> Result<(), ResolutionCode> {
    if s.is_empty() || s.contains('\0') {
        return Err(ResolutionCode::InvalidDescriptor {
            message: "empty or NUL token in entrypoint".into(),
        });
    }
    Ok(())
}

fn validate_env_key(k: &str) -> Result<(), ResolutionCode> {
    if k.is_empty()
        || k.contains('\0')
        || k.contains('=')
        || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(ResolutionCode::InvalidDescriptor {
            message: format!("invalid env key name `{k}`"),
        });
    }
    Ok(())
}

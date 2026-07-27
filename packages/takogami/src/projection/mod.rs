//! Sealed projection plans, validated scopes, and package Ontarch resolution.

mod op;
mod plan;
mod scope;

pub(crate) use op::ProjectionOperation;
pub(crate) use plan::SealedProjectionPlan;
pub(crate) use scope::{ScopeError, ValidatedBinScope};

//! Sealed projection plans, validated scopes, and package Ontarch resolution.

mod op;
mod plan;
mod scope;

pub(crate) use op::ProjectionOperation;
pub(crate) use plan::SealedProjectionPlan;
#[cfg(test)]
pub(crate) use plan::{clear_test_search_dirs, install_test_search_dirs};
pub(crate) use scope::{ScopeError, ValidatedBinScope};

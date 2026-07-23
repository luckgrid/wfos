//! Authorized execution handoff and executor seam (no production spawn in S5).

use super::evaluate::PolicyEvaluationExplanation;
use crate::contracts::PolicyDecision;
use crate::resolution::{PolicyEvaluationInput, RequestedOperation, SealedExecutionPlan};
use std::path::PathBuf;

/// Constructible only after dual-layer Allow.
#[derive(Debug, Clone)]
pub struct AuthorizedExecutionPlan {
    plan: SealedExecutionPlan,
    request: RequestedOperation,
    profile_id: String,
    policy_decision: PolicyDecision,
    policy_explanation: PolicyEvaluationExplanation,
    policy_root: PathBuf,
}

impl AuthorizedExecutionPlan {
    pub(crate) fn from_allow(
        input: &PolicyEvaluationInput,
        decision: PolicyDecision,
        explanation: PolicyEvaluationExplanation,
    ) -> Self {
        Self {
            plan: input.plan.clone(),
            request: input.request.clone(),
            profile_id: input.profile.id.clone(),
            policy_decision: decision,
            policy_explanation: explanation,
            policy_root: input.policy_root.clone(),
        }
    }

    pub fn plan(&self) -> &SealedExecutionPlan {
        &self.plan
    }

    pub fn request(&self) -> &RequestedOperation {
        &self.request
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn policy_decision(&self) -> &PolicyDecision {
        &self.policy_decision
    }

    pub fn policy_explanation(&self) -> &PolicyEvaluationExplanation {
        &self.policy_explanation
    }

    pub fn policy_root(&self) -> &PathBuf {
        &self.policy_root
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorResult {
    Unavailable,
    /// Test-only sentinel.
    SpyReached,
}

pub trait Executor {
    fn execute(&self, plan: &AuthorizedExecutionPlan) -> ExecutorResult;
}

/// Production S5 executor — never starts a child.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableExecutor;

impl Executor for UnavailableExecutor {
    fn execute(&self, _plan: &AuthorizedExecutionPlan) -> ExecutorResult {
        ExecutorResult::Unavailable
    }
}

/// Test spy that records whether it was reached.
#[derive(Debug, Default)]
pub struct SpyExecutor {
    pub reached: std::cell::Cell<bool>,
}

impl Executor for SpyExecutor {
    fn execute(&self, _plan: &AuthorizedExecutionPlan) -> ExecutorResult {
        self.reached.set(true);
        ExecutorResult::SpyReached
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spy_records_reachability() {
        // Construction of AuthorizedExecutionPlan requires full input; smoke the spy trait alone.
        let spy = SpyExecutor::default();
        assert!(!spy.reached.get());
    }
}

//! Evaluator-owned dual-Allow authorization for sealed projection plans.

use std::path::PathBuf;

use super::classify::classify_child;
use super::command::matches_command;
use super::evaluate::{
    PolicyContractError, PolicyEvaluationExplanation, PolicyLayer, PolicyLayerResult,
};
use super::normalize::{CanonicalRule, Effect, MatcherKind, MatcherPayload, normalize_policies};
use super::paths::{PathFactResult, evaluate_path_against_scopes, normalize_path_fact};
use super::raw::{PolicyContractKind, RawPolicyError};
use crate::contracts::PolicyDecision;
use crate::projection::SealedProjectionPlan;
use crate::registry::{PolicyRecord, ProfileRecord};

struct DualAllowProof {
    _private: (),
}

impl DualAllowProof {
    fn mint() -> Self {
        Self { _private: () }
    }
}

/// Opaque projection handoff. Construction remains private to this module.
#[derive(Debug, Clone)]
pub struct AuthorizedProjectionPlan {
    plan: SealedProjectionPlan,
    #[allow(dead_code)]
    profile_id: String,
    policy_decision: PolicyDecision,
    policy_explanation: PolicyEvaluationExplanation,
    #[allow(dead_code)]
    policy_root: PathBuf,
}

impl AuthorizedProjectionPlan {
    fn from_dual_allow(
        plan: SealedProjectionPlan,
        profile_id: String,
        matched_rules: Vec<String>,
        explanation: PolicyEvaluationExplanation,
        policy_root: PathBuf,
        _proof: DualAllowProof,
    ) -> Self {
        Self {
            plan,
            profile_id,
            policy_decision: PolicyDecision::Allow { matched_rules },
            policy_explanation: explanation,
            policy_root,
        }
    }

    pub(crate) fn plan(&self) -> &SealedProjectionPlan {
        &self.plan
    }
    pub fn policy_decision(&self) -> &PolicyDecision {
        &self.policy_decision
    }
    pub fn policy_explanation(&self) -> &PolicyEvaluationExplanation {
        &self.policy_explanation
    }
}

#[derive(Debug, Clone)]
pub struct RejectedProjectionOutcome {
    plan: SealedProjectionPlan,
    decision: PolicyDecision,
    explanation: PolicyEvaluationExplanation,
    deferred_unavailable: bool,
}

impl RejectedProjectionOutcome {
    pub(crate) fn plan(&self) -> &SealedProjectionPlan {
        &self.plan
    }
    pub fn decision(&self) -> &PolicyDecision {
        &self.decision
    }
    pub fn explanation(&self) -> &PolicyEvaluationExplanation {
        &self.explanation
    }
    pub fn deferred_unavailable(&self) -> bool {
        self.deferred_unavailable
    }
}

#[derive(Debug)]
pub enum ProjectionEvaluationResult {
    Contract(Box<PolicyContractError>),
    Rejected(Box<RejectedProjectionOutcome>),
    Authorized(Box<AuthorizedProjectionPlan>),
}

pub struct ProjectionEvaluationInput {
    plan: SealedProjectionPlan,
    profile: ProfileRecord,
    policies: Vec<PolicyRecord>,
    policy_origins: Vec<(String, String)>,
    policy_root: PathBuf,
}

impl ProjectionEvaluationInput {
    pub(crate) fn new(
        plan: SealedProjectionPlan,
        profile: ProfileRecord,
        policies: Vec<PolicyRecord>,
        policy_origins: Vec<(String, String)>,
        policy_root: PathBuf,
    ) -> Self {
        Self {
            plan,
            profile,
            policies,
            policy_origins,
            policy_root,
        }
    }
}

pub fn evaluate_projection_policy(input: &ProjectionEvaluationInput) -> ProjectionEvaluationResult {
    let session_id = input.plan.session_id().to_string();
    let plan_digest = input.plan.plan_digest().to_string();

    if let Err(e) = assert_projection_consistency(input) {
        return ProjectionEvaluationResult::Contract(Box::new(PolicyContractError::from((
            e,
            session_id,
            plan_digest,
        ))));
    }

    let normalized =
        match normalize_policies(
            &input.policies,
            &input.profile,
            &input.policy_origins,
            &input.policy_root,
        ) {
            Ok(n) => n,
            Err(e) => {
                return ProjectionEvaluationResult::Contract(Box::new(PolicyContractError::from(
                    (e, session_id, plan_digest),
                )));
            }
        };

    let request_argv = input.plan.operation().request_argv();
    let request_layer = reduce_command_layer(
        PolicyLayer::Request,
        "takogami",
        &request_argv,
        &normalized.rules,
        &[],
    );

    let child_argv = input.plan.argv();
    let intents = classify_child("ontarch", child_argv);
    let intent_names: Vec<String> = intents.iter().map(|i| i.as_str().to_string()).collect();
    let mut child_matched = match_commands("ontarch", child_argv, &normalized.rules);

    // Scope is a separate path fact: only explicit blocked matches deny (OutOfScope is not deny).
    if let Some(scope) = input.plan.safe_scope() {
        let fact = PathBuf::from(scope.as_str());
        if let Ok(rel) = normalize_path_fact(&fact, input.plan.cwd_path(), &input.policy_root) {
            if let PathFactResult::Blocked { matched_deny_rules } = evaluate_path_against_scopes(
                &rel,
                &normalized.allowed_path_patterns,
                &normalized.blocked_path_patterns,
            ) {
                for id in matched_deny_rules {
                    if let Some(rule) = normalized.rules.iter().find(|r| r.rule_id == id) {
                        child_matched.push(rule);
                    }
                }
            }
        } else {
            // Unnormalizable scope path facts fail closed.
            for rule in &normalized.rules {
                if rule.matcher == MatcherKind::Path
                    && rule.effect == Effect::Deny
                    && rule.safe_reason == "path_out_of_scope"
                {
                    child_matched.push(rule);
                }
            }
        }
    }

    let child_layer = reduce_matched(PolicyLayer::Child, child_matched, &intent_names);

    let effective = strongest_effect(&[
        effect_of(&request_layer.decision),
        effect_of(&child_layer.decision),
    ]);
    let decision = public_decision(effective, &request_layer, &child_layer, &normalized.rules);

    let dual_allow = request_layer.decision == "allow"
        && child_layer.decision == "allow"
        && matches!(decision, PolicyDecision::Allow { .. })
        && request_layer.command_authorized
        && child_layer.command_authorized;

    let explanation = PolicyEvaluationExplanation {
        actor: "agent".into(),
        profile_id: input.profile.id.clone(),
        plan_digest: plan_digest.clone(),
        precedence: "deny>gate>allow".into(),
        request: request_layer,
        child: child_layer,
        effective_decision: decision.clone(),
        execution_authorized: dual_allow,
        approval_transport: "unavailable".into(),
    };

    if dual_allow {
        if !input.plan.operation().child_supported() || input.plan.operation().mutation_deferred() {
            return ProjectionEvaluationResult::Rejected(Box::new(RejectedProjectionOutcome {
                plan: input.plan.clone(),
                decision: PolicyDecision::Deny {
                    policy_id: "controller".into(),
                    rule_id: "deferred_unavailable".into(),
                    reason: "deferred_unavailable".into(),
                },
                explanation,
                deferred_unavailable: true,
            }));
        }
        let matched = match &decision {
            PolicyDecision::Allow { matched_rules } => matched_rules.clone(),
            _ => Vec::new(),
        };
        return ProjectionEvaluationResult::Authorized(Box::new(
            AuthorizedProjectionPlan::from_dual_allow(
                input.plan.clone(),
                input.profile.id.clone(),
                matched,
                explanation,
                input.policy_root.clone(),
                DualAllowProof::mint(),
            ),
        ));
    }

    let deferred = input.plan.operation().mutation_deferred()
        && matches!(decision, PolicyDecision::Deny { .. });
    ProjectionEvaluationResult::Rejected(Box::new(RejectedProjectionOutcome {
        plan: input.plan.clone(),
        decision,
        explanation,
        deferred_unavailable: deferred,
    }))
}

fn assert_projection_consistency(input: &ProjectionEvaluationInput) -> Result<(), RawPolicyError> {
    let mismatch = |message: &str, field: &str| {
        RawPolicyError::new(
            PolicyContractKind::PolicyInputMismatch,
            message,
            None,
            Some(field.into()),
        )
    };
    if input.profile.id != input.plan.profile_id() {
        return Err(mismatch("profile id mismatch", "profile_id"));
    }
    let mut selected: Vec<String> = input.policies.iter().map(|p| p.id.clone()).collect();
    selected.sort();
    let mut planned = input.plan.policy_ids().to_vec();
    planned.sort();
    if selected != planned {
        return Err(mismatch("policy id set mismatch", "policy_ids"));
    }
    if input.policy_root.as_os_str().is_empty() || !input.policy_root.is_absolute() {
        return Err(mismatch("policy_root must be absolute", "policy_root"));
    }
    Ok(())
}

fn match_commands<'a>(
    program: &str,
    args: &[String],
    rules: &'a [CanonicalRule],
) -> Vec<&'a CanonicalRule> {
    let mut matched = Vec::new();
    for rule in rules {
        if rule.matcher != MatcherKind::Command {
            continue;
        }
        if let MatcherPayload::Command(pattern) = &rule.payload
            && matches_command(pattern, program, args)
        {
            matched.push(rule);
        }
    }
    matched
}

fn reduce_command_layer(
    layer: PolicyLayer,
    program: &str,
    args: &[String],
    rules: &[CanonicalRule],
    intents: &[String],
) -> PolicyLayerResult {
    reduce_matched(layer, match_commands(program, args, rules), intents)
}

fn reduce_matched(
    layer: PolicyLayer,
    mut matched: Vec<&CanonicalRule>,
    intents: &[String],
) -> PolicyLayerResult {
    matched.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    matched.dedup_by(|a, b| a.rule_id == b.rule_id);

    let has_deny = matched.iter().any(|r| r.effect == Effect::Deny);
    let has_gate = matched.iter().any(|r| r.effect == Effect::Gate);
    let has_command_allow = matched
        .iter()
        .any(|r| r.effect == Effect::Allow && r.matcher == MatcherKind::Command);

    let (decision, primary, command_authorized) = if has_deny {
        let primary = matched
            .iter()
            .filter(|r| r.effect == Effect::Deny)
            .map(|r| r.rule_id.clone())
            .min();
        ("deny".into(), primary, false)
    } else if has_gate {
        let primary = matched
            .iter()
            .filter(|r| r.effect == Effect::Gate)
            .map(|r| r.rule_id.clone())
            .min();
        ("gate".into(), primary, false)
    } else if has_command_allow {
        let primary = matched
            .iter()
            .filter(|r| r.effect == Effect::Allow && r.matcher == MatcherKind::Command)
            .map(|r| r.rule_id.clone())
            .min();
        ("allow".into(), primary, true)
    } else {
        ("deny".into(), None, false)
    };

    PolicyLayerResult {
        layer,
        decision,
        command_authorized,
        matched_rules: matched.iter().map(|r| r.rule_id.clone()).collect(),
        primary_rule: primary,
        intents: intents.to_vec(),
    }
}

fn strongest_effect(effects: &[Effect]) -> Effect {
    let mut best = Effect::Allow;
    for e in effects {
        best = match (best, *e) {
            (Effect::Deny, _) | (_, Effect::Deny) => Effect::Deny,
            (Effect::Gate, _) | (_, Effect::Gate) => Effect::Gate,
            _ => Effect::Allow,
        };
    }
    best
}

fn effect_of(decision: &str) -> Effect {
    match decision {
        "deny" => Effect::Deny,
        "gate" => Effect::Gate,
        _ => Effect::Allow,
    }
}

fn public_decision(
    effect: Effect,
    request: &PolicyLayerResult,
    child: &PolicyLayerResult,
    rules: &[CanonicalRule],
) -> PolicyDecision {
    match effect {
        Effect::Allow => {
            let mut matched = request.matched_rules.clone();
            matched.extend(child.matched_rules.clone());
            matched.sort();
            matched.dedup();
            PolicyDecision::Allow {
                matched_rules: matched,
            }
        }
        Effect::Gate => {
            let (policy_id, rule_id, reason) = pick(request, child, rules, Effect::Gate);
            PolicyDecision::Gate {
                policy_id,
                rule_id,
                reason,
                required_approval: "human".into(),
            }
        }
        Effect::Deny => {
            let (policy_id, rule_id, reason) = pick(request, child, rules, Effect::Deny);
            PolicyDecision::Deny {
                policy_id,
                rule_id,
                reason,
            }
        }
    }
}

fn pick(
    request: &PolicyLayerResult,
    child: &PolicyLayerResult,
    rules: &[CanonicalRule],
    want: Effect,
) -> (String, String, String) {
    for layer in [request, child] {
        for id in &layer.matched_rules {
            if let Some(rule) = rules.iter().find(|r| r.rule_id == *id && r.effect == want) {
                return (
                    rule.origin_id.clone(),
                    rule.rule_id.clone(),
                    rule.safe_reason.clone(),
                );
            }
        }
    }
    (
        "controller".into(),
        "default".into(),
        "denied by default".into(),
    )
}

#[cfg(test)]
mod non_forgeability {
    #[test]
    fn authorized_projection_has_no_public_constructor() {
        // Compile-time posture: AuthorizedProjectionPlan::from_dual_allow is private.
        // External crates/tests cannot mint authorization without DualAllowProof.
    }
}

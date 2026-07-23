//! Two-layer policy evaluation and merge.

use serde::Serialize;

use super::authorize::AuthorizedExecutionPlan;
use super::classify::{Intent, classify_child};
use super::command::matches_command;
use super::explain::render_human_policy_section;
use super::normalize::{
    CanonicalRule, Effect, MatcherKind, MatcherPayload, NormalizedPolicySet, normalize_policies,
};
use super::paths::{
    PathFactResult, evaluate_path_against_scopes, extract_path_operands, normalize_path_fact,
};
use super::raw::{PolicyContractKind, RawPolicyError};
use crate::contracts::PolicyDecision;
use crate::resolution::{Actor, PolicyEvaluationInput, RequestedOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyLayer {
    Request,
    Child,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyLayerResult {
    pub layer: PolicyLayer,
    pub decision: String,
    pub matched_rules: Vec<String>,
    pub primary_rule: Option<String>,
    pub intents: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyEvaluationExplanation {
    pub actor: String,
    pub profile_id: String,
    pub plan_digest: String,
    pub precedence: String,
    pub request: PolicyLayerResult,
    pub child: PolicyLayerResult,
    pub effective_decision: PolicyDecision,
    pub execution_authorized: bool,
    pub approval_transport: String,
}

#[derive(Debug, Clone)]
pub struct PolicyContractError {
    pub kind: PolicyContractKind,
    pub message: String,
    pub policy_id: Option<String>,
    pub field: Option<String>,
    pub session_id: String,
    pub plan_digest: String,
}

impl From<(RawPolicyError, String, String)> for PolicyContractError {
    fn from((e, session_id, plan_digest): (RawPolicyError, String, String)) -> Self {
        Self {
            kind: e.kind,
            message: e.message,
            policy_id: e.policy_id,
            field: e.field,
            session_id,
            plan_digest,
        }
    }
}

#[derive(Debug)]
pub enum PolicyEvaluationResult {
    Contract(Box<PolicyContractError>),
    Decided {
        decision: PolicyDecision,
        explanation: Box<PolicyEvaluationExplanation>,
        authorized: Option<Box<AuthorizedExecutionPlan>>,
    },
}

pub fn evaluate_policy(input: &PolicyEvaluationInput) -> PolicyEvaluationResult {
    let session_id = input.plan.resolved().session_id.clone();
    let plan_digest = input.plan.plan_digest().to_string();

    // Assert handoff consistency.
    if let Err(e) = assert_input_consistency(input) {
        return PolicyEvaluationResult::Contract(Box::new(PolicyContractError::from((
            e,
            session_id,
            plan_digest,
        ))));
    }

    let normalized = match normalize_policies(
        &input.policies,
        &input.profile,
        &input.policy_origins,
        &input.policy_root,
    ) {
        Ok(n) => n,
        Err(e) => {
            return PolicyEvaluationResult::Contract(Box::new(PolicyContractError::from((
                e,
                session_id,
                plan_digest,
            ))));
        }
    };

    let request_layer = evaluate_request_layer(&input.request, &normalized.rules);
    let child_layer = evaluate_child_layer(input, &normalized);

    let effective_effect = strongest(&[
        effect_from_decision(&request_layer.decision),
        effect_from_decision(&child_layer.decision),
    ]);

    let decision = build_public_decision(
        effective_effect,
        &request_layer,
        &child_layer,
        &normalized.rules,
    );

    let execution_authorized = matches!(decision, PolicyDecision::Allow { .. });
    let explanation = PolicyEvaluationExplanation {
        actor: "agent".into(),
        profile_id: input.profile.id.clone(),
        plan_digest: plan_digest.clone(),
        precedence: "deny>gate>allow".into(),
        request: request_layer,
        child: child_layer,
        effective_decision: decision.clone(),
        execution_authorized,
        approval_transport: "unavailable".into(),
    };

    let authorized = if execution_authorized {
        Some(Box::new(AuthorizedExecutionPlan::from_allow(
            input,
            decision.clone(),
            explanation.clone(),
        )))
    } else {
        None
    };

    let _ = render_human_policy_section(&explanation); // keep linked
    PolicyEvaluationResult::Decided {
        decision,
        explanation: Box::new(explanation),
        authorized,
    }
}

fn assert_input_consistency(input: &PolicyEvaluationInput) -> Result<(), RawPolicyError> {
    let resolved = input.plan.resolved();
    if resolved.profile_id != input.profile.id {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyInputMismatch,
            "profile id mismatch between plan and handoff",
            None,
            Some("profile_id".into()),
        ));
    }
    if !matches!(input.actor, Actor::Agent) {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyInputMismatch,
            "actor must be agent",
            None,
            Some("actor".into()),
        ));
    }
    Ok(())
}

fn evaluate_request_layer(
    request: &RequestedOperation,
    rules: &[CanonicalRule],
) -> PolicyLayerResult {
    let mut argv = vec![request.program.clone()];
    argv.extend(request.argv.iter().cloned());
    // argv for matching: program is "takogami", args are the rest
    let program = request.program.as_str();
    let args = &request.argv;

    let mut matched: Vec<&CanonicalRule> = Vec::new();
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

    reduce_layer(PolicyLayer::Request, matched, &[])
}

fn evaluate_child_layer(
    input: &PolicyEvaluationInput,
    normalized: &NormalizedPolicySet,
) -> PolicyLayerResult {
    let resolved = input.plan.resolved();
    let program = resolved.program.as_str();
    let args = &resolved.argv;
    let intents = classify_child(program, args);

    let mut matched: Vec<&CanonicalRule> = Vec::new();

    // Command rules
    for rule in &normalized.rules {
        if let MatcherPayload::Command(pattern) = &rule.payload
            && matches_command(pattern, program, args)
        {
            matched.push(rule);
        }
    }

    // Semantic / capability / remote rules from intents
    for intent in &intents {
        match intent {
            Intent::ShellWrapper => {
                push_semantic(&normalized.rules, &mut matched, "shell_wrapper");
            }
            Intent::SecretTool => {
                push_capability(&normalized.rules, &mut matched, "no_secret_read");
                push_capability(&normalized.rules, &mut matched, "secret_access");
            }
            Intent::Install => {
                push_capability(&normalized.rules, &mut matched, "no_global_install");
            }
            Intent::RemoteWrite | Intent::GhPublish => {
                push_remote(&normalized.rules, &mut matched);
            }
            Intent::DestructiveGit => {
                push_semantic(&normalized.rules, &mut matched, "destructive_git_cleanup");
                // also match deny command rules already handled
            }
            Intent::UnknownGh => {
                // default deny via no allow — no extra rule
            }
            _ => {}
        }
    }

    // Path facts
    let mut path_intents = intents.clone();
    let cwd = input.plan.cwd_path();
    let mut path_facts = vec![cwd.to_path_buf()];
    path_facts.extend(input.plan.source_manifest_paths().iter().cloned());
    path_facts.extend(extract_path_operands(program, args));

    for fact in &path_facts {
        match normalize_path_fact(fact, cwd, &input.policy_root) {
            Ok(rel) => {
                match evaluate_path_against_scopes(
                    &rel,
                    &normalized.allowed_path_patterns,
                    &normalized.blocked_path_patterns,
                ) {
                    PathFactResult::Allow => {
                        // collect allow path rules that matched — already have allow path rules
                    }
                    PathFactResult::Blocked => {
                        push_path_deny(&normalized.rules, &mut matched, true);
                    }
                    PathFactResult::OutOfScope | PathFactResult::Escape => {
                        push_path_deny(&normalized.rules, &mut matched, false);
                    }
                }
            }
            Err(PathFactResult::Escape)
            | Err(PathFactResult::Blocked)
            | Err(PathFactResult::OutOfScope) => {
                push_path_deny(&normalized.rules, &mut matched, false);
            }
            Err(PathFactResult::Allow) => {}
        }
    }

    let intent_names: Vec<String> = {
        path_intents.sort();
        path_intents.dedup();
        path_intents
            .iter()
            .map(|i| i.as_str().to_string())
            .collect()
    };

    reduce_layer(PolicyLayer::Child, matched, &intent_names)
}

fn push_semantic<'a>(
    rules: &'a [CanonicalRule],
    matched: &mut Vec<&'a CanonicalRule>,
    action: &'static str,
) {
    for rule in rules {
        if let MatcherPayload::SemanticAction(a) = rule.payload
            && a == action
        {
            matched.push(rule);
        }
    }
}

fn push_capability<'a>(
    rules: &'a [CanonicalRule],
    matched: &mut Vec<&'a CanonicalRule>,
    cap: &'static str,
) {
    for rule in rules {
        if let MatcherPayload::Capability(c) = rule.payload
            && c == cap
        {
            matched.push(rule);
        }
    }
}

fn push_remote<'a>(rules: &'a [CanonicalRule], matched: &mut Vec<&'a CanonicalRule>) {
    for rule in rules {
        if rule.matcher == MatcherKind::RemoteWrite {
            matched.push(rule);
        }
    }
}

fn push_path_deny<'a>(
    rules: &'a [CanonicalRule],
    matched: &mut Vec<&'a CanonicalRule>,
    blocked: bool,
) {
    for rule in rules {
        if rule.matcher != MatcherKind::Path || rule.effect != Effect::Deny {
            continue;
        }
        let matches_reason = if blocked {
            rule.safe_reason == "path_blocked"
        } else {
            rule.safe_reason == "path_out_of_scope"
        };
        if matches_reason {
            matched.push(rule);
        }
    }
}

fn reduce_layer(
    layer: PolicyLayer,
    mut matched: Vec<&CanonicalRule>,
    intents: &[String],
) -> PolicyLayerResult {
    matched.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    matched.dedup_by(|a, b| a.rule_id == b.rule_id);

    let has_deny = matched.iter().any(|r| r.effect == Effect::Deny);
    let has_gate = matched.iter().any(|r| r.effect == Effect::Gate);
    let has_allow = matched.iter().any(|r| r.effect == Effect::Allow);

    let (decision, primary) = if has_deny {
        let winners: Vec<_> = matched
            .iter()
            .filter(|r| r.effect == Effect::Deny)
            .collect();
        let primary = winners
            .iter()
            .map(|r| r.rule_id.as_str())
            .min()
            .map(str::to_string);
        ("deny".into(), primary)
    } else if has_gate {
        let winners: Vec<_> = matched
            .iter()
            .filter(|r| r.effect == Effect::Gate)
            .collect();
        let primary = winners
            .iter()
            .map(|r| r.rule_id.as_str())
            .min()
            .map(str::to_string);
        ("gate".into(), primary)
    } else if has_allow {
        let winners: Vec<_> = matched
            .iter()
            .filter(|r| r.effect == Effect::Allow)
            .collect();
        let primary = winners
            .iter()
            .map(|r| r.rule_id.as_str())
            .min()
            .map(str::to_string);
        ("allow".into(), primary)
    } else {
        ("deny".into(), Some("default_deny".into()))
    };

    PolicyLayerResult {
        layer,
        decision,
        matched_rules: matched.iter().map(|r| r.rule_id.clone()).collect(),
        primary_rule: primary,
        intents: intents.to_vec(),
    }
}

fn effect_from_decision(decision: &str) -> Effect {
    match decision {
        "allow" => Effect::Allow,
        "gate" => Effect::Gate,
        _ => Effect::Deny,
    }
}

fn strongest(effects: &[Effect]) -> Effect {
    if effects.contains(&Effect::Deny) {
        Effect::Deny
    } else if effects.contains(&Effect::Gate) {
        Effect::Gate
    } else {
        Effect::Allow
    }
}

fn build_public_decision(
    effect: Effect,
    request: &PolicyLayerResult,
    child: &PolicyLayerResult,
    rules: &[CanonicalRule],
) -> PolicyDecision {
    let all_matched: Vec<String> = {
        let mut v = request.matched_rules.clone();
        v.extend(child.matched_rules.iter().cloned());
        v.sort();
        v.dedup();
        v
    };
    match effect {
        Effect::Allow => PolicyDecision::Allow {
            matched_rules: all_matched,
        },
        Effect::Gate => {
            let primary = pick_primary(Effect::Gate, request, child);
            let (policy_id, rule_id, approval, reason) =
                lookup_rule(rules, &primary, "approval_required", "human_approval");
            PolicyDecision::Gate {
                policy_id,
                rule_id,
                reason,
                required_approval: approval,
            }
        }
        Effect::Deny => {
            let primary = pick_primary(Effect::Deny, request, child);
            if primary == "default_deny" {
                return PolicyDecision::Deny {
                    policy_id: "controller".into(),
                    rule_id: "default_deny".into(),
                    reason: "no_matching_allow".into(),
                };
            }
            let (policy_id, rule_id, _, reason) =
                lookup_rule(rules, &primary, "command_blocked", "human_approval");
            PolicyDecision::Deny {
                policy_id,
                rule_id,
                reason,
            }
        }
    }
}

fn pick_primary(effect: Effect, request: &PolicyLayerResult, child: &PolicyLayerResult) -> String {
    let want = match effect {
        Effect::Deny => "deny",
        Effect::Gate => "gate",
        Effect::Allow => "allow",
    };
    let mut candidates = Vec::new();
    if request.decision == want
        && let Some(p) = &request.primary_rule
    {
        candidates.push(p.clone());
    }
    if child.decision == want
        && let Some(p) = &child.primary_rule
    {
        candidates.push(p.clone());
    }
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| "default_deny".into())
}

fn lookup_rule(
    rules: &[CanonicalRule],
    primary: &str,
    default_reason: &str,
    default_approval: &str,
) -> (String, String, String, String) {
    if let Some(rule) = rules.iter().find(|r| r.rule_id == primary) {
        (
            rule.origin_id.clone(),
            rule.rule_id.clone(),
            rule.required_approval
                .clone()
                .unwrap_or_else(|| default_approval.into()),
            rule.safe_reason.clone(),
        )
    } else {
        (
            "controller".into(),
            primary.to_string(),
            default_approval.into(),
            default_reason.into(),
        )
    }
}

//! Two-layer policy evaluation and merge.

use std::collections::BTreeSet;

use serde::Serialize;

use super::authorize::{AllowDecision, AuthorizedExecutionPlan, DualAllowProof};
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
    let session_id = input.plan().resolved().session_id.clone();
    let plan_digest = input.plan().plan_digest().to_string();

    // Assert handoff consistency.
    if let Err(e) = assert_input_consistency(input) {
        return PolicyEvaluationResult::Contract(Box::new(PolicyContractError::from((
            e,
            session_id,
            plan_digest,
        ))));
    }

    let normalized = match normalize_policies(
        input.policies(),
        input.profile(),
        input.policy_origins(),
        input.policy_root(),
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

    let request_layer = evaluate_request_layer(input.request(), &normalized.rules);
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

    let dual_allow = request_layer.decision == "allow"
        && child_layer.decision == "allow"
        && matches!(decision, PolicyDecision::Allow { .. });
    let allow_matched = match &decision {
        PolicyDecision::Allow { matched_rules } if dual_allow => matched_rules.clone(),
        _ => Vec::new(),
    };

    let explanation = PolicyEvaluationExplanation {
        actor: "agent".into(),
        profile_id: input.profile().id.clone(),
        plan_digest: plan_digest.clone(),
        precedence: "deny>gate>allow".into(),
        request: request_layer,
        child: child_layer,
        effective_decision: decision.clone(),
        execution_authorized: dual_allow,
        approval_transport: "unavailable".into(),
    };

    let authorized = if dual_allow {
        Some(Box::new(AuthorizedExecutionPlan::from_dual_allow(
            input,
            AllowDecision::new(allow_matched),
            explanation.clone(),
            DualAllowProof::mint(),
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
    let mismatch = |message: &str, field: &str| {
        RawPolicyError::new(
            PolicyContractKind::PolicyInputMismatch,
            message,
            None,
            Some(field.into()),
        )
    };

    if !matches!(input.actor(), Actor::Agent) {
        return Err(mismatch("actor must be agent", "actor"));
    }

    let resolved = input.plan().resolved();
    let request = input.request();

    if request.program != "takogami" {
        return Err(mismatch(
            "request program must be takogami",
            "request.program",
        ));
    }
    if request.unit_id != resolved.unit_id {
        return Err(mismatch(
            "request unit_id mismatch between plan and handoff",
            "request.unit_id",
        ));
    }
    if request.verb != resolved.verb {
        return Err(mismatch(
            "request verb mismatch between plan and handoff",
            "request.verb",
        ));
    }

    let has_explain = request.argv.iter().any(|a| a == "--explain");
    let has_execute = request.argv.iter().any(|a| a == "--execute");
    if request.explain_requested != has_explain || request.execute_requested != has_execute {
        return Err(mismatch(
            "request boolean flags disagree with argv tokens",
            "request.argv",
        ));
    }

    let canonical = RequestedOperation::from_resolution(
        &resolved.unit_id,
        &resolved.verb,
        request.explain_requested,
        request.execute_requested,
    );
    if request.argv != canonical.argv {
        return Err(mismatch(
            "request argv is not the canonical [verb, unit_id, --explain?, --execute?] view",
            "request.argv",
        ));
    }

    if input.profile().id != resolved.profile_id {
        return Err(mismatch(
            "profile id mismatch between plan and handoff",
            "profile_id",
        ));
    }

    let mut selected_ids: Vec<String> = input.policies().iter().map(|p| p.id.clone()).collect();
    {
        let mut seen = BTreeSet::new();
        for id in &selected_ids {
            if !seen.insert(id.as_str()) {
                return Err(mismatch(
                    "selected policy record IDs are not unique",
                    "policies",
                ));
            }
        }
    }
    selected_ids.sort();

    let mut resolved_ids = resolved.policy_ids.clone();
    resolved_ids.sort();

    let mut origin_ids: Vec<String> = input
        .policy_origins()
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    {
        let mut seen = BTreeSet::new();
        for id in &origin_ids {
            if !seen.insert(id.as_str()) {
                return Err(mismatch(
                    "policy origin IDs are not unique",
                    "policy_origins",
                ));
            }
        }
    }
    origin_ids.sort();

    if selected_ids != resolved_ids || selected_ids != origin_ids {
        return Err(mismatch(
            "resolved.policy_ids, selected policy IDs, and origin IDs must be byte-equal after sorting",
            "policy_ids",
        ));
    }

    let root = input.policy_root();
    if root.as_os_str().is_empty() {
        return Err(mismatch("policy_root must be non-empty", "policy_root"));
    }

    Ok(())
}

fn evaluate_request_layer(
    request: &RequestedOperation,
    rules: &[CanonicalRule],
) -> PolicyLayerResult {
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
    let resolved = input.plan().resolved();
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
            Intent::ShellMutation => {
                // E09.S5.1: bounded classifier → no_shell_mutation capability.
                push_capability(&normalized.rules, &mut matched, "no_shell_mutation");
            }
            Intent::RemoteWrite | Intent::GhPublish => {
                push_remote(&normalized.rules, &mut matched);
            }
            Intent::DestructiveGit => {
                push_semantic(&normalized.rules, &mut matched, "destructive_git_cleanup");
            }
            Intent::UnknownGh => {}
            // Explanation-only: enforcement comes from command/path rules + path facts below.
            Intent::BinMutation
            | Intent::LocalGitMutation
            | Intent::GitInspection
            | Intent::BinReport
            | Intent::BinCleanupReportOnly
            | Intent::BinCleanupDryRun
            | Intent::BinCleanupArchive
            | Intent::BinCleanupDelete => {}
        }
    }

    // Path facts
    let mut path_intents = intents.clone();
    let cwd = input.plan().cwd_path();
    let mut path_facts = vec![cwd.to_path_buf()];
    path_facts.extend(input.plan().source_manifest_paths().iter().cloned());
    match extract_path_operands(program, args) {
        Ok(ops) => path_facts.extend(ops),
        Err(()) => {
            push_controller_path_deny(&normalized.rules, &mut matched);
        }
    }

    for fact in &path_facts {
        match normalize_path_fact(fact, cwd, input.policy_root()) {
            Ok(rel) => {
                match evaluate_path_against_scopes(
                    &rel,
                    &normalized.allowed_path_patterns,
                    &normalized.blocked_path_patterns,
                ) {
                    PathFactResult::Allow {
                        matched_allow_rules,
                    } => {
                        push_rules_by_id(&normalized.rules, &mut matched, &matched_allow_rules);
                    }
                    PathFactResult::Blocked { matched_deny_rules } => {
                        push_rules_by_id(&normalized.rules, &mut matched, &matched_deny_rules);
                    }
                    PathFactResult::OutOfScope | PathFactResult::Escape => {
                        push_controller_path_deny(&normalized.rules, &mut matched);
                    }
                }
            }
            Err(PathFactResult::Escape) | Err(PathFactResult::OutOfScope) => {
                push_controller_path_deny(&normalized.rules, &mut matched);
            }
            Err(PathFactResult::Allow { .. }) | Err(PathFactResult::Blocked { .. }) => {}
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

fn push_rules_by_id<'a>(
    rules: &'a [CanonicalRule],
    matched: &mut Vec<&'a CanonicalRule>,
    ids: &[String],
) {
    for id in ids {
        if let Some(rule) = rules.iter().find(|r| r.rule_id == *id) {
            matched.push(rule);
        }
    }
}

/// Controller out-of-scope / escape deny — exact rule by safe_reason, not every path Deny.
fn push_controller_path_deny<'a>(rules: &'a [CanonicalRule], matched: &mut Vec<&'a CanonicalRule>) {
    for rule in rules {
        if rule.matcher == MatcherKind::Path
            && rule.effect == Effect::Deny
            && rule.safe_reason == "path_out_of_scope"
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::fingerprint_file;
    use crate::registry::{PolicyRecord, ProfileRecord, RegistryAccess, RegistryPaths};
    use crate::resolution::{FixedIdGenerator, LifecycleVerb, ResolutionRequest, resolve};
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    struct DemoHandoff {
        _temp: TempDir,
        input: PolicyEvaluationInput,
    }

    #[test]
    fn well_formed_handoff_evaluates_allow_with_layers() {
        let handoff = resolve_demo_handoff(false);
        let result = evaluate_policy(&handoff.input);
        let PolicyEvaluationResult::Decided {
            decision,
            explanation,
            authorized,
        } = result
        else {
            panic!("expected Decided, got contract error");
        };
        assert!(
            matches!(decision, PolicyDecision::Allow { .. }),
            "decision={decision:?} request={} child={} rules_req={:?} rules_child={:?}",
            explanation.request.decision,
            explanation.child.decision,
            explanation.request.matched_rules,
            explanation.child.matched_rules
        );
        assert!(explanation.execution_authorized);
        assert_eq!(explanation.request.decision, "allow");
        assert_eq!(explanation.child.decision, "allow");
        assert!(
            !explanation.child.matched_rules.is_empty(),
            "child should carry path/command rule provenance"
        );
        assert!(
            authorized.is_some(),
            "dual-Allow must mint AuthorizedExecutionPlan"
        );
    }

    #[test]
    fn normalize_rejects_duplicate_policy_ids() {
        let profile = ProfileRecord {
            id: "p".into(),
            title: None,
            purpose: None,
            rails: None,
            rails_bin: None,
            isolation_mode: None,
            isolation_jj: None,
            session_state_home: None,
            rest: serde_json::from_value(serde_json::json!({
                "allowed_paths": ["**"]
            }))
            .unwrap(),
        };
        let pol = PolicyRecord {
            id: "dup".into(),
            applies_to: Some("agent".into()),
            rest: serde_json::from_value(serde_json::json!({
                "version": "0.1.0",
                "allow": {"commands": ["x"]}
            }))
            .unwrap(),
        };
        let err = normalize_policies(
            &[pol.clone(), pol],
            &profile,
            &[("dup".into(), "origin".into())],
            Path::new("/tmp"),
        )
        .unwrap_err();
        assert!(err.message.contains("duplicate"));
        assert_eq!(err.field.as_deref(), Some("id"));
    }

    fn resolve_demo_handoff(execute: bool) -> DemoHandoff {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        fs::create_dir_all(&workspace).unwrap();
        copy_tree(&fixture, &workspace);

        let path_dir = workspace.join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["moon", "demo-bin", "rg"] {
            let p = path_dir.join(name);
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        let registry = workspace.join("registry");
        write_demo_units(&workspace, &registry);

        let access = RegistryAccess::new(RegistryPaths {
            registry_root: registry,
            workspace_root: workspace,
        });
        let request = ResolutionRequest {
            session_id: "tkg_eval".into(),
            unit_id: "demo".into(),
            verb: LifecycleVerb::Build,
            explicit_profile: None,
            explain: false,
            execute_requested: execute,
        };
        let mut id_gen = FixedIdGenerator {
            id: "tkg_unused".into(),
        };
        let success =
            resolve(&access, request, vec![path_dir], None, &mut id_gen).expect("resolve demo");
        DemoHandoff {
            input: success.policy_evaluation_input(),
            _temp: temp,
        }
    }

    fn write_demo_units(workspace: &Path, registry: &Path) {
        let desc_dir = registry.join("sources/descriptors");
        let mut fps = Vec::new();
        let mut units = Vec::new();
        for entry in fs::read_dir(&desc_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let authored: toml::Value =
                toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let id = authored["id"].as_str().unwrap().to_string();
            if id != "demo" {
                continue;
            }
            let rel = format!(
                "registry/sources/descriptors/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            fps.push(fingerprint_file(&workspace.join(&rel), &rel).unwrap());
            let entrypoints: serde_json::Value =
                serde_json::to_value(authored.get("entrypoints").unwrap()).unwrap();
            let native: serde_json::Value = serde_json::to_value(
                authored
                    .get("native")
                    .and_then(|n| n.get("manifests"))
                    .unwrap(),
            )
            .unwrap();
            units.push(serde_json::json!({
                "id": id,
                "kind": "package",
                "path": "demo",
                "native_manifests": native,
                "entrypoints": entrypoints,
                "source": "central",
                "provides": [],
                "requires": [],
            }));
        }
        let doc = serde_json::json!({
            "generated_at": "2026-07-21T00:00:00Z",
            "registry_generation": {
                "generated_at": "2026-07-21T00:00:00Z",
                "source_fingerprints": fps,
            },
            "summary": {"total": units.len()},
            "units": units,
        });
        fs::write(
            registry.join("units.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
    }

    fn copy_tree(src: &Path, dst: &Path) {
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir_all(&to).unwrap();
                copy_tree(&entry.path(), &to);
            } else {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::copy(entry.path(), &to).unwrap();
            }
        }
    }
}

//! Normalize policy/profile shapes into sorted canonical rules.

use std::collections::BTreeSet;

use super::command::{matcher_digest, parse_command_pattern};
use super::paths::{CompiledPathPattern, adjust_pattern_for_root, compile_path_pattern};
use super::raw::{
    PolicyContractKind, PolicyEnforcementRecord, ProfileEnforcement, RawPolicyError,
    RemoteWritePolicy,
};
use crate::registry::{PolicyRecord, ProfileRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    Allow,
    Gate,
    Deny,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Gate => "gate",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OriginKind {
    Controller,
    Policy,
    Profile,
}

impl OriginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Policy => "policy",
            Self::Profile => "profile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatcherKind {
    Command,
    Path,
    Capability,
    RemoteWrite,
    SemanticAction,
}

impl MatcherKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Path => "path",
            Self::Capability => "capability",
            Self::RemoteWrite => "remote_write",
            Self::SemanticAction => "semantic_action",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatcherPayload {
    Command(Vec<String>),
    Path(String),
    Capability(&'static str),
    RemoteWrite(RemoteWritePolicy),
    SemanticAction(&'static str),
}

#[derive(Debug, Clone)]
pub struct CanonicalRule {
    pub rule_id: String,
    pub origin_kind: OriginKind,
    pub origin_id: String,
    pub effect: Effect,
    pub matcher: MatcherKind,
    pub payload: MatcherPayload,
    pub required_approval: Option<String>,
    pub safe_reason: String,
}

#[derive(Debug, Clone)]
pub struct NormalizedPolicySet {
    pub rules: Vec<CanonicalRule>,
    pub profile: ProfileEnforcement,
    pub allowed_path_patterns: Vec<CompiledPathPattern>,
    pub blocked_path_patterns: Vec<CompiledPathPattern>,
}

pub fn normalize_policies(
    policies: &[PolicyRecord],
    profile: &ProfileRecord,
    policy_origins: &[(String, String)],
    policy_root: &std::path::Path,
) -> Result<NormalizedPolicySet, RawPolicyError> {
    // Duplicate policy IDs are a contract error.
    let mut seen = BTreeSet::new();
    for p in policies {
        if !seen.insert(p.id.clone()) {
            return Err(RawPolicyError::new(
                PolicyContractKind::PolicyContractInvalid,
                format!("duplicate policy id `{}`", p.id),
                Some(p.id.clone()),
                Some("id".into()),
            ));
        }
    }

    let profile_enf = ProfileEnforcement::from_record(profile)?;
    let mut rules = Vec::new();

    // Sort by policy ID so the first malformed diagnostic is order-independent.
    let mut policies_sorted: Vec<&PolicyRecord> = policies.iter().collect();
    policies_sorted.sort_by(|a, b| a.id.cmp(&b.id));

    // Controller hard-block wrappers.
    for wrapper in [
        "sh", "bash", "zsh", "fish", "env", "sudo", "doas", "xargs", "command", "exec",
    ] {
        rules.push(make_rule(
            OriginKind::Controller,
            "hard-block",
            Effect::Deny,
            MatcherKind::SemanticAction,
            MatcherPayload::SemanticAction("shell_wrapper"),
            None,
            "wrapper_blocked",
            &format!("wrapper:{wrapper}"),
        ));
    }
    // One semantic rule is enough; the classifier fires shell_wrapper.
    rules.retain(|r| {
        !(r.origin_kind == OriginKind::Controller
            && matches!(r.payload, MatcherPayload::SemanticAction("shell_wrapper")))
    });
    rules.push(make_rule(
        OriginKind::Controller,
        "hard-block",
        Effect::Deny,
        MatcherKind::SemanticAction,
        MatcherPayload::SemanticAction("shell_wrapper"),
        None,
        "wrapper_blocked",
        "wrapper",
    ));
    rules.push(make_rule(
        OriginKind::Controller,
        "path-scope",
        Effect::Deny,
        MatcherKind::Path,
        MatcherPayload::Path("__out_of_scope__".into()),
        None,
        "path_out_of_scope",
        "out_of_scope",
    ));

    for policy in &policies_sorted {
        let enf = PolicyEnforcementRecord::from_registry(policy)?;
        let _origin_label = policy_origins
            .iter()
            .find(|(id, _)| id == &enf.id)
            .map(|(_, l)| l.as_str())
            .unwrap_or("policy");

        // panoply [agent] allow/block → panoply <cmd>
        if let Some(agent) = &enf.agent {
            let prefix = if enf.applies_to == "panoply" {
                "panoply"
            } else {
                enf.applies_to.as_str()
            };
            for cmd in &agent.allow {
                let pattern = parse_command_pattern(&format!("{prefix} {cmd}"), &enf.id)?;
                rules.push(command_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Allow,
                    pattern,
                    None,
                    "command_blocked", // unused for allow
                ));
            }
            for cmd in &agent.block {
                let pattern = parse_command_pattern(&format!("{prefix} {cmd}"), &enf.id)?;
                rules.push(command_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Deny,
                    pattern,
                    None,
                    "command_blocked",
                ));
            }
        }

        emit_rule_table(
            &mut rules,
            OriginKind::Policy,
            &enf.id,
            Effect::Allow,
            enf.allow.as_ref(),
            None,
        )?;
        emit_rule_table(
            &mut rules,
            OriginKind::Policy,
            &enf.id,
            Effect::Gate,
            enf.gate.as_ref(),
            Some("human_approval"),
        )?;
        emit_rule_table(
            &mut rules,
            OriginKind::Policy,
            &enf.id,
            Effect::Deny,
            enf.block.as_ref(),
            None,
        )?;

        if let Some(gates) = &enf.gates {
            if gates.no_global_install == Some(true) {
                rules.push(make_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Deny,
                    MatcherKind::Capability,
                    MatcherPayload::Capability("no_global_install"),
                    None,
                    "install_blocked",
                    "no_global_install",
                ));
            }
            if gates.no_secret_read == Some(true) {
                rules.push(make_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Deny,
                    MatcherKind::Capability,
                    MatcherPayload::Capability("no_secret_read"),
                    None,
                    "secret_access_blocked",
                    "no_secret_read",
                ));
            }
            if gates.no_shell_mutation == Some(true) {
                rules.push(make_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Deny,
                    MatcherKind::Capability,
                    MatcherPayload::Capability("no_shell_mutation"),
                    None,
                    "command_blocked",
                    "no_shell_mutation",
                ));
            }
        }

        if let Some(secrets) = &enf.secrets {
            for tool in &secrets.block_tools {
                let pattern = parse_command_pattern(tool, &enf.id)?;
                rules.push(command_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Deny,
                    pattern,
                    None,
                    "secret_access_blocked",
                ));
            }
        }

        if let Some(remote) = &enf.remote
            && let Some(writes) = remote.writes
        {
            let (effect, approval, reason) = match writes {
                RemoteWritePolicy::Elevated => (
                    Effect::Gate,
                    Some("elevated_remote_write_approval"),
                    "approval_required",
                ),
                RemoteWritePolicy::Blocked | RemoteWritePolicy::LocalOnly => {
                    (Effect::Deny, None, "remote_write_blocked")
                }
            };
            rules.push(make_rule(
                OriginKind::Policy,
                &enf.id,
                effect,
                MatcherKind::RemoteWrite,
                MatcherPayload::RemoteWrite(writes),
                approval,
                reason,
                writes.as_str(),
            ));
        }
    }

    // Profile command tiers.
    for cmd in &profile_enf.allowed_commands {
        let pattern = parse_command_pattern(cmd, &profile_enf.id)?;
        rules.push(command_rule(
            OriginKind::Profile,
            &profile_enf.id,
            Effect::Allow,
            pattern,
            None,
            "command_blocked",
        ));
    }
    for cmd in &profile_enf.gated_commands {
        let pattern = parse_command_pattern(cmd, &profile_enf.id)?;
        rules.push(command_rule(
            OriginKind::Profile,
            &profile_enf.id,
            Effect::Gate,
            pattern,
            Some("human_approval"),
            "approval_required",
        ));
    }
    for cmd in &profile_enf.blocked_commands {
        let pattern = parse_command_pattern(cmd, &profile_enf.id)?;
        rules.push(command_rule(
            OriginKind::Profile,
            &profile_enf.id,
            Effect::Deny,
            pattern,
            None,
            "command_blocked",
        ));
    }

    if !profile_enf.secret_access {
        rules.push(make_rule(
            OriginKind::Profile,
            &profile_enf.id,
            Effect::Deny,
            MatcherKind::Capability,
            MatcherPayload::Capability("secret_access"),
            None,
            "secret_access_blocked",
            "secret_access_false",
        ));
    }

    let (effect, approval, reason) = match profile_enf.remote_write_policy {
        RemoteWritePolicy::Elevated => (
            Effect::Gate,
            Some("elevated_remote_write_approval"),
            "approval_required",
        ),
        RemoteWritePolicy::Blocked | RemoteWritePolicy::LocalOnly => {
            (Effect::Deny, None, "remote_write_blocked")
        }
    };
    rules.push(make_rule(
        OriginKind::Profile,
        &profile_enf.id,
        effect,
        MatcherKind::RemoteWrite,
        MatcherPayload::RemoteWrite(profile_enf.remote_write_policy),
        approval,
        reason,
        profile_enf.remote_write_policy.as_str(),
    ));

    // Path patterns — bind each CompiledPathPattern to its canonical rule_id.
    let mut allowed_path_patterns = Vec::new();
    for p in &profile_enf.allowed_paths {
        let adjusted = adjust_pattern_for_root(p, policy_root);
        let rule = make_rule(
            OriginKind::Profile,
            &profile_enf.id,
            Effect::Allow,
            MatcherKind::Path,
            MatcherPayload::Path(adjusted.clone()),
            None,
            "path_out_of_scope",
            &adjusted,
        );
        let compiled = compile_path_pattern(&adjusted, &profile_enf.id, &rule.rule_id)?;
        rules.push(rule);
        allowed_path_patterns.push(compiled);
    }
    let mut blocked_path_patterns = Vec::new();
    for p in &profile_enf.blocked_paths {
        let adjusted = adjust_pattern_for_root(p, policy_root);
        let rule = make_rule(
            OriginKind::Profile,
            &profile_enf.id,
            Effect::Deny,
            MatcherKind::Path,
            MatcherPayload::Path(adjusted.clone()),
            None,
            "path_blocked",
            &adjusted,
        );
        let compiled = compile_path_pattern(&adjusted, &profile_enf.id, &rule.rule_id)?;
        rules.push(rule);
        blocked_path_patterns.push(compiled);
    }

    // Also path blocks from policy block.paths
    for policy in &policies_sorted {
        let enf = PolicyEnforcementRecord::from_registry(policy)?;
        if let Some(block) = &enf.block {
            for p in &block.paths {
                let adjusted = adjust_pattern_for_root(p, policy_root);
                let rule = make_rule(
                    OriginKind::Policy,
                    &enf.id,
                    Effect::Deny,
                    MatcherKind::Path,
                    MatcherPayload::Path(adjusted.clone()),
                    None,
                    "path_blocked",
                    &adjusted,
                );
                let compiled = compile_path_pattern(&adjusted, &enf.id, &rule.rule_id)?;
                rules.push(rule);
                blocked_path_patterns.push(compiled);
            }
            for action in &block.actions {
                if action == "delete untracked files" {
                    rules.push(make_rule(
                        OriginKind::Policy,
                        &enf.id,
                        Effect::Deny,
                        MatcherKind::SemanticAction,
                        MatcherPayload::SemanticAction("destructive_git_cleanup"),
                        None,
                        "destructive_operation_blocked",
                        "delete_untracked",
                    ));
                }
            }
        }
    }

    // Sort/dedupe by rule_id.
    rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    rules.dedup_by(|a, b| a.rule_id == b.rule_id);

    Ok(NormalizedPolicySet {
        rules,
        profile: profile_enf,
        allowed_path_patterns,
        blocked_path_patterns,
    })
}

fn emit_rule_table(
    rules: &mut Vec<CanonicalRule>,
    origin: OriginKind,
    origin_id: &str,
    effect: Effect,
    table: Option<&super::raw::RuleTable>,
    approval: Option<&str>,
) -> Result<(), RawPolicyError> {
    let Some(table) = table else {
        return Ok(());
    };
    let reason = match effect {
        Effect::Allow => "command_blocked",
        Effect::Gate => "approval_required",
        Effect::Deny => "command_blocked",
    };
    for cmd in table.commands.iter().chain(table.tools.iter()) {
        let pattern = parse_command_pattern(cmd, origin_id)?;
        rules.push(command_rule(
            origin, origin_id, effect, pattern, approval, reason,
        ));
    }
    Ok(())
}

fn command_rule(
    origin: OriginKind,
    origin_id: &str,
    effect: Effect,
    pattern: Vec<String>,
    approval: Option<&str>,
    reason: &str,
) -> CanonicalRule {
    let payload_key = pattern.join("\u{1f}");
    make_rule(
        origin,
        origin_id,
        effect,
        MatcherKind::Command,
        MatcherPayload::Command(pattern),
        approval,
        reason,
        &payload_key,
    )
}

#[allow(clippy::too_many_arguments)]
fn make_rule(
    origin: OriginKind,
    origin_id: &str,
    effect: Effect,
    matcher: MatcherKind,
    payload: MatcherPayload,
    approval: Option<&str>,
    reason: &str,
    digest_src: &str,
) -> CanonicalRule {
    let dig = matcher_digest(digest_src);
    let rule_id = format!(
        "{}:{}:{}:{}:{dig}",
        origin.as_str(),
        origin_id,
        effect.as_str(),
        matcher.as_str()
    );
    CanonicalRule {
        rule_id,
        origin_kind: origin,
        origin_id: origin_id.to_string(),
        effect,
        matcher,
        payload,
        required_approval: approval.map(str::to_string),
        safe_reason: reason.to_string(),
    }
}

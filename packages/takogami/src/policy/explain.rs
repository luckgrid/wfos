//! Safe policy explanation rendering.

use super::evaluate::PolicyEvaluationExplanation;

pub fn render_human_policy_section(explanation: &PolicyEvaluationExplanation) -> String {
    let mut lines = Vec::new();
    lines.push("Policy:".to_string());
    lines.push(format!("  actor: {}", explanation.actor));
    lines.push(format!("  profile: {}", explanation.profile_id));
    lines.push("  precedence: deny > gate > allow".to_string());
    lines.push(format!("  request: {}", explanation.request.decision));
    lines.push(format!("  child: {}", explanation.child.decision));
    let effective = match &explanation.effective_decision {
        crate::contracts::PolicyDecision::Allow { .. } => "allow",
        crate::contracts::PolicyDecision::Gate { .. } => "gate",
        crate::contracts::PolicyDecision::Deny { .. } => "deny",
    };
    lines.push(format!("  effective: {effective}"));
    let mut rules = explanation.request.matched_rules.clone();
    rules.extend(explanation.child.matched_rules.iter().cloned());
    rules.sort();
    rules.dedup();
    if !rules.is_empty() {
        lines.push(format!("  matched rules: {}", rules.join(", ")));
    }
    lines.push(format!(
        "  execution authorized: {}",
        if explanation.execution_authorized {
            "yes"
        } else {
            "no"
        }
    ));
    lines.push(format!(
        "  approval transport: {}",
        explanation.approval_transport
    ));
    lines.join("\n")
}

pub fn render_human_policy_summary(explanation: &PolicyEvaluationExplanation) -> String {
    let n = {
        let mut rules = explanation.request.matched_rules.clone();
        rules.extend(explanation.child.matched_rules.iter().cloned());
        rules.sort();
        rules.dedup();
        rules.len()
    };
    let effective = match &explanation.effective_decision {
        crate::contracts::PolicyDecision::Allow { .. } => "Allow",
        crate::contracts::PolicyDecision::Gate { .. } => "Gate",
        crate::contracts::PolicyDecision::Deny { .. } => "Deny",
    };
    format!(
        "Policy: {effective} (request={}, child={}; {n} rules)",
        explanation.request.decision, explanation.child.decision
    )
}

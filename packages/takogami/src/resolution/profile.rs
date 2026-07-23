//! Profile selection and policy-reference collection.

use std::collections::{BTreeMap, BTreeSet};

use super::entrypoint::NormalizedEntrypoint;
use super::resolver::ResolutionCode;
use crate::registry::{PoliciesDocument, PolicyRecord, ProfileRecord, ProfilesDocument};

#[derive(Debug, Clone)]
pub struct SelectedProfile {
    pub profile: ProfileRecord,
    pub policies: Vec<PolicyRecord>,
    pub policy_ids: Vec<String>,
    pub policy_origins: Vec<(String, String)>,
}

pub fn select_profile(
    docs: &ProfilesDocument,
    explicit: Option<&str>,
    env_profile: Option<&str>,
) -> Result<ProfileRecord, ResolutionCode> {
    let id = if let Some(p) = explicit.filter(|s| !s.is_empty()) {
        p.to_string()
    } else if let Some(p) = env_profile.filter(|s| !s.is_empty()) {
        p.to_string()
    } else if docs.profiles.iter().any(|p| p.id == "workspace-dev") {
        "workspace-dev".to_string()
    } else {
        return Err(ResolutionCode::ProfileRequired);
    };

    let matches: Vec<_> = docs.profiles.iter().filter(|p| p.id == id).collect();
    match matches.as_slice() {
        [] => Err(ResolutionCode::ProfileNotFound { id }),
        [one] => Ok((*one).clone()),
        _ => Err(ResolutionCode::ProfileAmbiguous { id }),
    }
}

pub fn collect_policy_refs(
    policies: &PoliciesDocument,
    profile: &ProfileRecord,
    entry: &NormalizedEntrypoint,
    unit_id: &str,
) -> Result<SelectedProfile, ResolutionCode> {
    // Reject duplicate source IDs before map insertion can silently drop a body.
    let mut seen_ids = BTreeSet::new();
    for p in &policies.policies {
        if !seen_ids.insert(p.id.as_str()) {
            return Err(ResolutionCode::PolicyDuplicate { id: p.id.clone() });
        }
    }

    let mut origins: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    let push = |map: &mut BTreeMap<String, BTreeSet<String>>, id: &str, origin: &str| {
        map.entry(id.to_string())
            .or_default()
            .insert(origin.to_string());
    };

    for p in &policies.policies {
        if p.applies_to.as_deref() == Some("agent") {
            push(&mut origins, &p.id, "global agent policy");
        }
    }
    if let Some(rails) = profile.rails.as_deref() {
        push(&mut origins, rails, "profile rails");
    }
    if let Some(rails_bin) = profile.rails_bin.as_deref() {
        push(&mut origins, rails_bin, "profile rails_bin");
    }
    for id in &entry.required_policies {
        push(&mut origins, id, "entrypoint");
    }
    for p in &policies.policies {
        if p.applies_to.as_deref() == Some(unit_id) {
            push(&mut origins, &p.id, "unit policy");
        }
    }

    let by_id: BTreeMap<&str, &PolicyRecord> = policies
        .policies
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    for id in origins.keys() {
        if !by_id.contains_key(id.as_str()) {
            return Err(ResolutionCode::PolicyNotFound { id: id.clone() });
        }
    }

    let policy_ids: Vec<String> = origins.keys().cloned().collect();
    let selected_policies = policy_ids
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).map(|policy| (*policy).clone()))
        .collect();
    let policy_origins: Vec<(String, String)> = origins
        .iter()
        .map(|(id, labels)| {
            let mut sorted: Vec<_> = labels.iter().cloned().collect();
            sorted.sort();
            (id.clone(), sorted.join(", "))
        })
        .collect();

    Ok(SelectedProfile {
        profile: profile.clone(),
        policies: selected_policies,
        policy_ids,
        policy_origins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ExecutionClass;
    use crate::registry::{PoliciesDocument, PolicyRecord, ProfileRecord};

    #[test]
    fn duplicate_policy_ids_rejected_at_collect() {
        let policies = PoliciesDocument {
            generated_at: "t".into(),
            policies: vec![
                PolicyRecord {
                    id: "a".into(),
                    applies_to: Some("agent".into()),
                    rest: Default::default(),
                },
                PolicyRecord {
                    id: "a".into(),
                    applies_to: Some("agent".into()),
                    rest: Default::default(),
                },
            ],
        };
        let profile = ProfileRecord {
            id: "p".into(),
            title: None,
            purpose: None,
            rails: None,
            rails_bin: None,
            isolation_mode: None,
            isolation_jj: None,
            session_state_home: None,
            rest: Default::default(),
        };
        let entry = NormalizedEntrypoint {
            program: "moon".into(),
            args: vec![],
            cwd: None,
            env_keys: vec![],
            backend: Some("native".into()),
            adapter: Some("direct".into()),
            source_manifests: vec![],
            required_policies: vec![],
            execution_class: ExecutionClass::Direct,
            runtime_provider: None,
            diagnostics: vec![],
        };
        assert!(matches!(
            collect_policy_refs(&policies, &profile, &entry, "demo"),
            Err(ResolutionCode::PolicyDuplicate { id }) if id == "a"
        ));
    }
}

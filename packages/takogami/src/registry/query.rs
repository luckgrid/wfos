//! Bounded unit/tool list filters and info lookup.

use crate::error::ControllerError;

use super::types::{ToolRecord, UnitRecord};

const UNIT_FILTER_KEYS: &[&str] = &[
    "kind",
    "domain",
    "layer",
    "stack",
    "owner",
    "runtime",
    "status",
    "provides",
    "requires",
    // tools list also shares --filter
    "module",
    "id",
    "installed",
    "default",
];

#[derive(Debug, Clone, Default)]
pub struct UnitFilters {
    pub pairs: Vec<(String, String)>,
}

pub fn parse_filters(raw: &[String]) -> Result<UnitFilters, ControllerError> {
    let mut pairs = Vec::new();
    for item in raw {
        let Some((key, value)) = item.split_once('=') else {
            return Err(ControllerError::invalid_filter(format!(
                "filter must be FIELD=VALUE, got `{item}`"
            )));
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err(ControllerError::invalid_filter(format!(
                "filter must be FIELD=VALUE, got `{item}`"
            )));
        }
        if !UNIT_FILTER_KEYS.contains(&key) {
            return Err(ControllerError::invalid_filter(format!(
                "unsupported filter field `{key}`; allowed: {}",
                UNIT_FILTER_KEYS.join(", ")
            )));
        }
        pairs.push((key.to_string(), value.to_string()));
    }
    Ok(UnitFilters { pairs })
}

pub fn filter_units(units: &[UnitRecord], filters: &UnitFilters) -> Vec<UnitRecord> {
    units
        .iter()
        .filter(|u| filters.pairs.iter().all(|(k, v)| unit_matches(u, k, v)))
        .cloned()
        .collect()
}

fn unit_matches(unit: &UnitRecord, key: &str, value: &str) -> bool {
    match key {
        "kind" => unit.kind.as_deref() == Some(value),
        "domain" => unit.domain.as_deref() == Some(value),
        "layer" => unit.layer.as_deref() == Some(value),
        "stack" => unit.stack.as_deref() == Some(value),
        "owner" => unit.owner.as_deref() == Some(value),
        "runtime" => unit.runtime.as_deref() == Some(value),
        "status" => unit.status.as_deref() == Some(value),
        "provides" => unit.provides.iter().any(|p| p == value),
        "requires" => unit.requires.iter().any(|r| r == value),
        _ => false,
    }
}

pub fn find_unit<'a>(units: &'a [UnitRecord], id: &str) -> Result<&'a UnitRecord, ControllerError> {
    let matches: Vec<_> = units.iter().filter(|u| u.id == id).collect();
    match matches.as_slice() {
        [] => Err(ControllerError::not_found(format!("unit `{id}` not found"))),
        [one] => Ok(*one),
        many => Err(ControllerError::ambiguous(format!(
            "unit id `{id}` matches {} records: {}",
            many.len(),
            many.iter()
                .map(|u| u.path.as_deref().unwrap_or(&u.id))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

pub fn filter_tools(tools: &[ToolRecord], filters: &UnitFilters) -> Vec<ToolRecord> {
    tools
        .iter()
        .filter(|t| {
            filters.pairs.iter().all(|(k, v)| match k.as_str() {
                "module" => t.module.as_deref() == Some(v.as_str()),
                "id" => t.id == *v,
                "installed" => t.installed.map(|b| b.to_string()) == Some(v.clone()),
                "default" => t.default.map(|b| b.to_string()) == Some(v.clone()),
                // unit-oriented keys never match tool records
                _ => false,
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_filter_key() {
        let err = parse_filters(&["nope=1".into()]).unwrap_err();
        assert_eq!(err.diagnostic_code(), "invalid_filter");
    }

    #[test]
    fn filters_by_kind() {
        let units = vec![
            UnitRecord {
                id: "a".into(),
                kind: Some("workspace".into()),
                title: None,
                status: None,
                domain: None,
                layer: None,
                stack: None,
                owner: None,
                runtime: None,
                path: None,
                native_manifests: vec![],
                entrypoints: std::collections::BTreeMap::new(),
                cli: None,
                provides: vec![],
                requires: vec![],
                policy: None,
                source: None,
                provisional: false,
                routing_complete: None,
            },
            UnitRecord {
                id: "b".into(),
                kind: Some("package".into()),
                title: None,
                status: None,
                domain: None,
                layer: None,
                stack: None,
                owner: None,
                runtime: None,
                path: None,
                native_manifests: vec![],
                entrypoints: std::collections::BTreeMap::new(),
                cli: None,
                provides: vec![],
                requires: vec![],
                policy: None,
                source: None,
                provisional: false,
                routing_complete: None,
            },
        ];
        let f = parse_filters(&["kind=workspace".into()]).unwrap();
        let got = filter_units(&units, &f);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "a");
    }
}

//! E09.S7 Phase 1 direct Ontarch graph contracts (S7-R05).
//! Named tests for fingerprint/schema gates; hermetic via copied package tree.

#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use std::fs;
use support::{HermeticOntarch, snapshot_checkout_registry};

#[test]
fn ontarch_graph_direct_tests_do_not_write_checkout_registry() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    // Minimal upstream docs so sync/graph can be attempted without touching checkout.
    for name in [
        "units.json",
        "policies.json",
        "profiles.json",
        "skills.json",
    ] {
        fs::write(
            h.registry.join(name),
            format!(
                r#"{{"generated_at":"2026-07-25T00:00:00Z","{k}":[]}}"#,
                k = name.trim_end_matches(".json")
            ),
        )
        .unwrap();
    }
    let _ = h.run_script(&h.sync, &[]);
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_graph_contains_exact_upstream_fingerprints() {
    let h = HermeticOntarch::new();
    for name in [
        "units.json",
        "policies.json",
        "profiles.json",
        "skills.json",
    ] {
        fs::write(
            h.registry.join(name),
            format!(
                r#"{{"generated_at":"2026-07-25T00:00:00Z","{k}":[]}}"#,
                k = name.trim_end_matches(".json")
            ),
        )
        .unwrap();
    }
    let out = h.run_script(&h.sync, &[]);
    assert!(
        out.status.success(),
        "ontarch sync: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    let fps = graph["registry_generation"]["source_fingerprints"]
        .as_array()
        .expect("registry_generation.source_fingerprints required");
    let paths: Vec<&str> = fps.iter().filter_map(|f| f["path"].as_str()).collect();
    for required in [
        "registry/units.json",
        "registry/policies.json",
        "registry/profiles.json",
        "registry/skills.json",
    ] {
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with(required.trim_start_matches("registry/")) || *p == required),
            "missing fingerprint for {required}; got {paths:?}"
        );
    }
}

#[test]
fn ontarch_graph_fingerprint_paths_are_sorted_and_portable() {
    let h = HermeticOntarch::new();
    for name in [
        "units.json",
        "policies.json",
        "profiles.json",
        "skills.json",
    ] {
        fs::write(
            h.registry.join(name),
            format!(
                r#"{{"generated_at":"2026-07-25T00:00:00Z","{k}":[]}}"#,
                k = name.trim_end_matches(".json")
            ),
        )
        .unwrap();
    }
    assert!(h.run_script(&h.sync, &[]).status.success());
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    let paths: Vec<String> = graph["registry_generation"]["source_fingerprints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "fingerprint paths must be sorted");
}

#[test]
fn ontarch_graph_schema_rejects_missing_registry_generation() {
    let h = HermeticOntarch::new();
    fs::write(
        h.registry.join("graph.json"),
        r#"{"generated_at":"2026-07-25T00:00:00Z","nodes":[],"edges":[]}"#,
    )
    .unwrap();
    let out = h.run_script(&h.validate, &[]);
    assert!(
        !out.status.success(),
        "validate must reject graph without registry_generation"
    );
}

#[test]
fn ontarch_graph_schema_rejects_unknown_root_node_edge_fields() {
    let h = HermeticOntarch::new();
    fs::write(
        h.registry.join("graph.json"),
        r#"{
          "generated_at":"2026-07-25T00:00:00Z",
          "registry_generation":{"generated_at":"2026-07-25T00:00:00Z","source_fingerprints":[]},
          "nodes":[{"id":"a","kind":"package","extra":true}],
          "edges":[],
          "unexpected":1
        }"#,
    )
    .unwrap();
    let out = h.run_script(&h.validate, &[]);
    assert!(
        !out.status.success(),
        "validate must reject unknown fields under closed schema"
    );
}

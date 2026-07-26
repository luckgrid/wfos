//! JSON Schema + semantic helpers for Phase 1 bin machine contracts.

use super::payloads::sample_cleanup_plan;
use jsonschema::Validator;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

fn checkout_schema(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ontarch/schemas")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn inventory_validator() -> &'static Validator {
    static V: OnceLock<Validator> = OnceLock::new();
    V.get_or_init(|| {
        Validator::new(&checkout_schema("bin-inventory.schema.json"))
            .expect("compile bin-inventory schema")
    })
}

fn cleanup_validator() -> &'static Validator {
    static V: OnceLock<Validator> = OnceLock::new();
    V.get_or_init(|| {
        Validator::new(&checkout_schema("bin-cleanup-plan.schema.json"))
            .expect("compile bin-cleanup-plan schema")
    })
}

pub fn validate_inventory_schema(doc: &Value) {
    if let Err(err) = inventory_validator().validate(doc) {
        panic!("inventory schema validation failed: {err}\n{doc}");
    }
}

pub fn inventory_schema_ok(doc: &Value) -> bool {
    inventory_validator().validate(doc).is_ok()
}

pub fn validate_cleanup_schema(doc: &Value) {
    if let Err(err) = cleanup_validator().validate(doc) {
        panic!("cleanup schema validation failed: {err}\n{doc}");
    }
}

pub fn cleanup_schema_ok(doc: &Value) -> bool {
    cleanup_validator().validate(doc).is_ok()
}

pub fn assert_inventory_semantics(doc: &Value, expected_root: &str) {
    validate_inventory_schema(doc);
    assert_eq!(doc["root"], expected_root);
    let workflows = doc["workflows"].as_array().unwrap();
    assert_eq!(doc["summary"]["total"], workflows.len());
    let with_manifest = workflows
        .iter()
        .filter(|w| w["manifest_present"].as_bool() == Some(true))
        .count();
    assert_eq!(doc["summary"]["with_manifest"], with_manifest);

    let mut prev: Option<&str> = None;
    let mut seen = std::collections::BTreeSet::new();
    for w in workflows {
        let path = w["path"].as_str().unwrap();
        assert!(!path.starts_with('/'), "absolute path: {path}");
        assert!(!path.contains('\\'), "backslash path: {path}");
        assert!(!path.contains(".."), "traversal path: {path}");
        assert!(
            path.contains("/bin/"),
            "path must stay under namespace bin/: {path}"
        );
        assert!(
            !path.split('/').any(|s| s == "lib" || s == "src"),
            "lib/src path forbidden: {path}"
        );
        assert!(seen.insert(path), "duplicate path: {path}");
        if let Some(p) = prev {
            assert!(p < path, "unsorted: {p} before {path}");
        }
        prev = Some(path);

        let mc = w["manifest_count"].as_u64().unwrap();
        let present = w["manifest_present"].as_bool().unwrap();
        assert_eq!(present, mc > 0);
        assert!(w["size_bytes"].as_u64().unwrap() <= 1_099_511_627_776);
        if let (Some(oldest), Some(newest)) = (
            w["oldest_file_age_days"].as_u64(),
            w["newest_file_age_days"].as_u64(),
        ) {
            assert!(newest <= oldest, "newest > oldest for {path}");
        }
    }
}

pub fn assert_cleanup_semantics(doc: &Value) {
    validate_cleanup_schema(doc);
    assert_eq!(doc["mutation_executed"], false);
    let entries = doc["entries"].as_array().unwrap();
    assert_eq!(doc["summary"]["total"], entries.len());
    let mut counts = std::collections::BTreeMap::from([
        ("advisory", 0usize),
        ("would_archive", 0usize),
        ("would_delete", 0usize),
        ("blocked", 0usize),
    ]);
    let mut prev: Option<&str> = None;
    let mut seen = std::collections::BTreeSet::new();
    for e in entries {
        let path = e["path"].as_str().unwrap();
        assert!(!path.starts_with('/'));
        assert!(!path.contains('\\'));
        assert!(!path.contains(".."));
        assert!(seen.insert(path), "duplicate {path}");
        if let Some(p) = prev {
            assert!(p < path);
        }
        prev = Some(path);
        let disp = e["disposition"].as_str().unwrap();
        *counts.get_mut(disp).unwrap() += 1;
        if let Some(scope) = doc["scope"].as_str() {
            assert!(
                path == scope || path.starts_with(&format!("{scope}/")),
                "entry {path} outside scope {scope}"
            );
        }
    }
    assert_eq!(doc["summary"]["advisory"], counts["advisory"]);
    assert_eq!(doc["summary"]["would_archive"], counts["would_archive"]);
    assert_eq!(doc["summary"]["would_delete"], counts["would_delete"]);
    assert_eq!(doc["summary"]["blocked"], counts["blocked"]);
}

/// One-mutation fixtures: Rust schema and production shell validators must agree.
pub fn inventory_one_mutation_corpus(root: &str) -> Vec<(&'static str, Value)> {
    let mut good = sample_inventory_one(root);
    let mut out = Vec::new();
    let mut frac_size = good.clone();
    frac_size["workflows"][0]["size_bytes"] = json!(1.5);
    out.push(("inventory fractional size", frac_size));
    let mut frac_count = good.clone();
    frac_count["workflows"][0]["file_count"] = json!(1.5);
    out.push(("inventory fractional count", frac_count));
    let mut neg_age = good.clone();
    neg_age["workflows"][0]["oldest_file_age_days"] = json!(-1);
    out.push(("inventory negative age", neg_age));
    let mut over_max = good.clone();
    over_max["workflows"][0]["file_count"] = json!(10_000_001);
    out.push(("inventory over maximum", over_max));
    let mut obj_wf = good.clone();
    obj_wf["workflows"] = json!({"path": "Build/bin/demo"});
    out.push(("inventory workflow is object instead of array", obj_wf));
    let mut scalar_wf = good.clone();
    scalar_wf["workflows"] = json!("Build/bin/demo");
    out.push(("inventory workflow is scalar", scalar_wf));
    let _ = &mut good;
    out
}

pub fn cleanup_one_mutation_corpus() -> Vec<(&'static str, Value)> {
    let good = sample_cleanup_plan("report-only");
    let mut out = Vec::new();
    let mut entries_obj = good.clone();
    entries_obj["entries"] = json!({"path": "Build/bin/demo"});
    out.push(("cleanup entries is object", entries_obj));
    let mut frac = good.clone();
    frac["summary"]["total"] = json!(0.5);
    out.push(("cleanup fractional summary", frac));
    let mut neg = good.clone();
    neg["summary"]["blocked"] = json!(-1);
    out.push(("cleanup negative summary", neg));
    let mut bad_ret = good.clone();
    bad_ret["entries"] = json!([{
        "path": "Build/bin/demo",
        "disposition": "advisory",
        "reason": "current",
        "retention": 12,
        "approved_to_matches": null
    }]);
    bad_ret["summary"] = json!({
        "total": 1, "advisory": 1, "would_archive": 0, "would_delete": 0, "blocked": 0
    });
    out.push(("cleanup invalid retention type", bad_ret));
    let mut bad_am = good.clone();
    bad_am["entries"] = json!([{
        "path": "Build/bin/demo",
        "disposition": "advisory",
        "reason": "current",
        "retention": null,
        "approved_to_matches": "yes"
    }]);
    bad_am["summary"] = json!({
        "total": 1, "advisory": 1, "would_archive": 0, "would_delete": 0, "blocked": 0
    });
    out.push(("cleanup invalid approved_to_matches type", bad_am));
    let mut tab_path = good.clone();
    tab_path["entries"] = json!([{
        "path": "Build/bin/demo\t",
        "disposition": "advisory",
        "reason": "current",
        "retention": null,
        "approved_to_matches": null
    }]);
    tab_path["summary"] = json!({
        "total": 1, "advisory": 1, "would_archive": 0, "would_delete": 0, "blocked": 0
    });
    out.push(("cleanup tab/newline/control path", tab_path));
    let mut unsafe_scope = good.clone();
    unsafe_scope["scope"] = json!("../etc");
    out.push(("cleanup unsafe scope", unsafe_scope));
    let _ = good;
    out
}

fn sample_inventory_one(root: &str) -> Value {
    json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "root": root,
        "summary": {"total": 1, "with_manifest": 1},
        "workflows": [{
            "path": "Build/bin/demo",
            "size_bytes": 10,
            "file_count": 1,
            "oldest_file_age_days": 2,
            "newest_file_age_days": 1,
            "manifest_present": true,
            "manifest_count": 1
        }]
    })
}

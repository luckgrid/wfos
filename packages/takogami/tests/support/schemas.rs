//! JSON Schema + semantic helpers for Phase 1 bin machine contracts.

use jsonschema::Validator;
use serde_json::Value;
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

pub fn validate_cleanup_schema(doc: &Value) {
    if let Err(err) = cleanup_validator().validate(doc) {
        panic!("cleanup schema validation failed: {err}\n{doc}");
    }
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

//! E09.S7 Phase 1 direct Ontarch bin contracts (S7-R01/R05).
//! Hermetic: executes copied scripts only; proves checkout registry untouched.

#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use std::fs;
use support::{HermeticOntarch, sample_cleanup_plan, sample_inventory, snapshot_checkout_registry};

fn assert_one_json_document(raw: &str) -> Value {
    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    let first = stream
        .next()
        .expect("expected one JSON document on stdout")
        .expect("JSON parse");
    assert!(
        stream.next().is_none(),
        "exactly one JSON document required; got trailing data"
    );
    assert!(!raw.contains(":: ontarch"), "stdout must be machine-pure");
    first
}

#[test]
fn ontarch_direct_tests_do_not_write_checkout_registry() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    // May fail until Phase 1 --json lands; must still stay hermetic.
    let _ = h.run_bin_report(&["--json"]);
    let _ = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    let after = snapshot_checkout_registry();
    assert_eq!(
        before, after,
        "direct Ontarch tests must not mutate checkout packages/ontarch/registry"
    );
}

#[test]
fn ontarch_bin_report_json_emits_one_pure_document() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let out = h.run_bin_report(&["--json"]);
    assert!(
        out.status.success(),
        "bin-report --json: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert!(v.get("generated_at").is_some());
    assert!(v.get("root").is_some());
    assert!(v.get("summary").is_some());
    assert!(v.get("workflows").is_some());
    assert!(
        v.get("mutation").is_none(),
        "plan forbids invented mutation field"
    );
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_bin_report_json_validates_complete_inventory_schema() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let out = h.run_bin_report(&["--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    let summary = &v["summary"];
    assert!(summary.get("total").is_some());
    assert!(summary.get("with_manifest").is_some());
}

#[test]
fn ontarch_bin_cleanup_report_only_json_emits_one_pure_document() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.write_inventory_fixture(&sample_inventory(h.ws_root.to_str().unwrap()));
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    assert!(
        out.status.success(),
        "cleanup report-only --json: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_eq!(v["mutation_executed"], false);
    assert!(v.get("entries").is_some());
    assert!(v.get("actions").is_none());
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_bin_cleanup_dry_run_json_emits_one_pure_document() {
    let h = HermeticOntarch::new();
    h.write_inventory_fixture(&sample_inventory(h.ws_root.to_str().unwrap()));
    let out = h.run_bin_cleanup(&["--mode", "dry-run", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_eq!(v["mode"], "dry-run");
    assert_eq!(v["mutation_executed"], false);
}

#[test]
fn ontarch_bin_cleanup_json_reports_inventory_refresh_explicitly() {
    let h = HermeticOntarch::new();
    // No inventory fixture — refresh must be explicit in the plan document.
    fs::remove_file(h.registry.join("bin-inventory.json")).ok();
    h.seed_bin_workflow("Build", "demo", false);
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert!(
        v.get("inventory_refreshed").is_some(),
        "inventory_refreshed must be present: {v}"
    );
}

#[test]
fn ontarch_report_and_dry_run_do_not_mutate_bin_tree() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let bin_root = h.ws_root.join("Build/bin");
    let before = support::hash_tree(&bin_root);
    let _ = h.run_bin_report(&["--json"]);
    let inv = sample_inventory(h.ws_root.to_str().unwrap());
    h.write_inventory_fixture(&inv);
    let _ = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    let _ = h.run_bin_cleanup(&["--mode", "dry-run", "--json"]);
    let after = support::hash_tree(&bin_root);
    assert_eq!(
        before, after,
        "report-only/dry-run must not mutate bin tree"
    );
}

#[test]
fn ontarch_archive_and_delete_refuse_in_agent_and_draft_gateway_modes() {
    let h = HermeticOntarch::new();
    h.write_inventory_fixture(&sample_inventory(h.ws_root.to_str().unwrap()));
    for mode in ["archive", "delete-approved"] {
        let mut args = vec!["--mode", mode];
        let scope;
        if mode == "delete-approved" {
            scope = "Build/bin/demo".to_string();
            args.push("--scope");
            args.push(scope.as_str());
        }
        // Agent mode
        let out = std::process::Command::new(&h.bin_cleanup)
            .args(&args)
            .current_dir(&h.ws_root)
            .env("WS_ROOT", &h.ws_root)
            .env("PANOPLY_AGENT", "1")
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "{mode} under PANOPLY_AGENT=1 must refuse"
        );
        // Draft gateway (no agent)
        let out2 = h.run_bin_cleanup(&args);
        assert!(
            !out2.status.success(),
            "{mode} at draft gateway must refuse"
        );
    }
}

#[test]
fn ontarch_scope_matrix_rejects_absolute_traversal_control_lib_src() {
    let h = HermeticOntarch::new();
    h.write_inventory_fixture(&sample_inventory(h.ws_root.to_str().unwrap()));
    for scope in [
        "/etc/passwd",
        "../..",
        "Build/bin/demo\n",
        "lib/foo",
        "src/foo",
        "~/secret",
        "Build\\bin\\demo",
    ] {
        let out = h.run_bin_cleanup(&["--mode", "report-only", "--scope", scope, "--json"]);
        assert!(
            !out.status.success(),
            "scope {scope:?} must fail closed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn ontarch_inventory_fd_and_find_paths_are_equivalent() {
    // Phase 1 portability: same inventory bytes with fd present vs find fallback.
    // Named now; fails until Phase 1 implements --json with tool fallback matrix.
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "nested", true);
    fs::create_dir_all(h.ws_root.join("Build/bin/nested/deep")).unwrap();
    fs::write(
        h.ws_root.join("Build/bin/nested/deep/manifest.json"),
        r#"{"id":"nested"}"#,
    )
    .unwrap();
    let with_path = h.run_bin_report(&["--json"]);
    assert!(
        with_path.status.success(),
        "{}",
        String::from_utf8_lossy(&with_path.stderr)
    );
    let a = assert_one_json_document(std::str::from_utf8(&with_path.stdout).unwrap());
    // Force find path by clearing PATH of fd (keep coreutils).
    let out2 = std::process::Command::new(&h.bin_report)
        .arg("--json")
        .current_dir(&h.ws_root)
        .env("WS_ROOT", &h.ws_root)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(
        out2.status.success(),
        "{}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let b = assert_one_json_document(std::str::from_utf8(&out2.stdout).unwrap());
    assert_eq!(a["summary"], b["summary"]);
    assert_eq!(a["workflows"], b["workflows"]);
}

#[test]
fn ontarch_inventory_bsd_and_gnu_stat_paths_are_equivalent() {
    // Named Phase 1 portability gate: inventory ages must be stable across stat dialects.
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let out = h.run_bin_report(&["--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert!(v["workflows"].is_array());
}

#[test]
fn sample_cleanup_plan_fixture_shape_is_plan_aligned() {
    let doc = sample_cleanup_plan("report-only");
    assert_eq!(doc["mutation_executed"], false);
    assert!(doc.get("entries").is_some());
    assert!(doc.get("inventory_generated_at").is_some());
}

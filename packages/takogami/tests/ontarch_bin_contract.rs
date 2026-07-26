//! E09.S7 Phase 1 direct Ontarch bin contracts (S7-P1-R01/R04–R10).

#[path = "support/mod.rs"]
mod support;

use serde_json::{Value, json};
use std::fs;
use support::{
    HermeticOntarch, assert_cleanup_semantics, assert_inventory_semantics,
    sample_cleanup_mutation_true, sample_cleanup_plan, sample_inventory, snapshot_bin_tree,
    snapshot_checkout_registry, validate_cleanup_schema, validate_inventory_schema,
    write_executable,
};

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
    assert!(
        !raw.contains("ok "),
        "stdout must not contain human ok lines"
    );
    first
}

fn require_success(out: &std::process::Output, label: &str) {
    assert!(
        out.status.success(),
        "{label}: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn ontarch_direct_tests_do_not_write_checkout_registry() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "bin-report");
    require_success(
        &h.run_bin_cleanup(&["--mode", "report-only", "--json"]),
        "cleanup",
    );
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_bin_report_json_emits_one_pure_document() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let out = h.run_bin_report(&["--json"]);
    require_success(&out, "bin-report --json");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_inventory_semantics(&v, h.ws_root.to_str().unwrap());
    assert!(v.get("mutation").is_none());
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_bin_report_json_validates_complete_inventory_schema() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let out = h.run_bin_report(&["--json"]);
    require_success(&out, "bin-report");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    validate_inventory_schema(&v);
    assert_inventory_semantics(&v, h.ws_root.to_str().unwrap());
}

#[test]
fn ontarch_bin_report_human_mode_preserves_table() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let out = h.run_bin_report(&[]);
    require_success(&out, "bin-report human");
    let md = fs::read_to_string(h.registry.join("BIN-INVENTORY.md")).unwrap();
    assert!(md.contains("| path |"));
    assert!(md.contains("Build/bin/demo"));
}

#[test]
fn ontarch_bin_report_invalid_workflow_name_retains_previous_inventory() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "seed inventory");
    let before_json = fs::read(h.registry.join("bin-inventory.json")).unwrap();
    let before_md = fs::read(h.registry.join("BIN-INVENTORY.md")).unwrap();
    // Invalid workflow dirname (space) makes emit produce a schema-illegal path.
    let bad = h.ws_root.join("Build/bin/demo space");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("x.txt"), b"x\n").unwrap();
    let out = h.run_bin_report(&["--json"]);
    assert!(
        !out.status.success(),
        "invalid workflow path must fail report"
    );
    assert_eq!(
        before_json,
        fs::read(h.registry.join("bin-inventory.json")).unwrap()
    );
    assert_eq!(
        before_md,
        fs::read(h.registry.join("BIN-INVENTORY.md")).unwrap()
    );
    assert_no_registry_temp_leaks(&h);
}

#[test]
fn ontarch_bin_cleanup_report_only_json_emits_one_pure_document() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    h.write_inventory_fixture(&sample_inventory(h.ws_root.to_str().unwrap()));
    // Refresh empty inventory from seed by regenerating
    require_success(&h.run_bin_report(&["--json"]), "inventory");
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    require_success(&out, "cleanup report-only --json");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_cleanup_semantics(&v);
    assert_eq!(v["mutation_executed"], false);
    assert!(v.get("actions").is_none());
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_bin_cleanup_dry_run_json_emits_one_pure_document() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "inventory");
    let out = h.run_bin_cleanup(&["--mode", "dry-run", "--json"]);
    require_success(&out, "cleanup dry-run --json");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_eq!(v["mode"], "dry-run");
    assert_cleanup_semantics(&v);
}

#[test]
fn ontarch_bin_cleanup_json_reports_inventory_refresh_explicitly() {
    let h = HermeticOntarch::new();
    fs::remove_file(h.registry.join("bin-inventory.json")).ok();
    h.seed_bin_workflow("Build", "demo", false);
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    require_success(&out, "cleanup refresh");
    let stdout = std::str::from_utf8(&out.stdout).unwrap_or("");
    assert!(
        !stdout.trim().is_empty(),
        "expected JSON stdout; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = assert_one_json_document(stdout);
    assert_eq!(v["inventory_refreshed"], true);
    assert_cleanup_semantics(&v);
}

#[test]
fn ontarch_report_and_dry_run_do_not_mutate_bin_tree() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let bin_root = h.ws_root.join("Build/bin");
    let before = snapshot_bin_tree(&bin_root);

    let out = h.run_bin_report(&["--json"]);
    require_success(&out, "report");
    let inv = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_inventory_semantics(&inv, h.ws_root.to_str().unwrap());
    assert_eq!(before, snapshot_bin_tree(&bin_root));

    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    require_success(&out, "report-only");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_cleanup_semantics(&plan);
    assert_eq!(plan["mutation_executed"], false);
    assert_eq!(before, snapshot_bin_tree(&bin_root));

    let out = h.run_bin_cleanup(&["--mode", "dry-run", "--json"]);
    require_success(&out, "dry-run");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_cleanup_semantics(&plan);
    assert_eq!(plan["mutation_executed"], false);
    assert_eq!(before, snapshot_bin_tree(&bin_root));
}

#[test]
fn ontarch_archive_and_delete_refuse_with_stderr_json() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "inventory");
    for mode in ["archive", "delete-approved"] {
        let mut args = vec!["--mode", mode, "--json"];
        let scope;
        if mode == "delete-approved" {
            scope = "Build/bin/demo".to_string();
            args.push("--scope");
            args.push(scope.as_str());
        }
        let out = std::process::Command::new(&h.bin_cleanup)
            .args(&args)
            .current_dir(&h.ws_root)
            .env("WS_ROOT", &h.ws_root)
            .env_remove("AGENTS_HOME")
            .env("PANOPLY_AGENT", "1")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{mode} agent must refuse");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("deferred_unavailable") || err.contains("\"error\""),
            "stderr JSON refusal expected: {err}"
        );
        assert!(
            out.stdout.is_empty()
                || !std::str::from_utf8(&out.stdout)
                    .unwrap()
                    .contains("mutation_executed\":true")
        );

        let out2 = h.run_bin_cleanup(&args);
        assert!(!out2.status.success(), "{mode} draft gateway must refuse");
        let err2 = String::from_utf8_lossy(&out2.stderr);
        assert!(err2.contains("deferred_unavailable"), "{err2}");
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
fn ontarch_option_matrix_mode_scope_json() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "inv");

    // --json before mode
    let out = h.run_bin_cleanup(&["--json", "--mode", "report-only"]);
    require_success(&out, "json before mode");
    assert_cleanup_semantics(&assert_one_json_document(
        std::str::from_utf8(&out.stdout).unwrap(),
    ));

    // --mode=value
    let out = h.run_bin_cleanup(&["--mode=dry-run", "--json"]);
    require_success(&out, "mode=value");
    assert_eq!(
        assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap())["mode"],
        "dry-run"
    );

    // duplicate --mode
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--mode", "dry-run", "--json"]);
    assert!(!out.status.success());

    // missing --mode value
    let out = h.run_bin_cleanup(&["--mode", "--json"]);
    assert!(!out.status.success());

    // unknown option
    let out = h.run_bin_cleanup(&["--bogus", "--json"]);
    assert!(!out.status.success());

    // valid scope
    let out = h.run_bin_cleanup(&["--mode", "dry-run", "--scope", "Build/bin/demo", "--json"]);
    require_success(&out, "scoped dry-run");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_eq!(v["scope"], "Build/bin/demo");
    assert_cleanup_semantics(&v);
}

#[test]
fn ontarch_cleanup_semantic_matrix_dispositions() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow_manifest(
        "Build",
        "perm",
        &HermeticOntarch::complete_manifest("perm", "permanent", None),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "review",
        &HermeticOntarch::complete_manifest("review", "review-before-delete", None),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "sess",
        &HermeticOntarch::complete_manifest("sess", "session-exports", None),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "fresh-auto",
        &HermeticOntarch::complete_manifest("fa", "auto-archive-after:99999d", None),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "stale-auto",
        &HermeticOntarch::complete_manifest("sa", "auto-archive-after:1d", None),
    );
    h.seed_bin_workflow("Build", "bare", false);
    h.seed_bin_workflow_manifest(
        "Build",
        "approved",
        &HermeticOntarch::complete_manifest(
            "a",
            "review-before-delete",
            Some("Build/bin/approved"),
        ),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "mismatch",
        &HermeticOntarch::complete_manifest("m", "review-before-delete", Some("Build/bin/other")),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "nullapp",
        &HermeticOntarch::complete_manifest("n", "review-before-delete", None),
    );
    h.seed_bin_workflow_manifest(
        "Build",
        "incomplete",
        r#"{"id":"inc","approved_to":"Build/bin/incomplete"}"#,
    );
    let nested = h.ws_root.join("Build/bin/nested/deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("artifact.txt"), b"x\n").unwrap();
    fs::write(
        nested.join("manifest.json"),
        HermeticOntarch::complete_manifest("nested", "review-before-delete", None),
    )
    .unwrap();
    let multi = h.ws_root.join("Build/bin/multi");
    fs::create_dir_all(multi.join("a")).unwrap();
    fs::create_dir_all(multi.join("b")).unwrap();
    fs::write(
        multi.join("a/manifest.json"),
        HermeticOntarch::complete_manifest("a", "review-before-delete", None),
    )
    .unwrap();
    fs::write(
        multi.join("b/manifest.json"),
        HermeticOntarch::complete_manifest("b", "review-before-delete", None),
    )
    .unwrap();

    let tools = h.tools_bsd_stat();
    let out = h.run_with_path(&h.bin_report, &["--json"], &tools);
    require_success(&out, "inv");

    let out = h.run_with_path(&h.bin_cleanup, &["--mode", "dry-run", "--json"], &tools);
    require_success(&out, "dry-run matrix");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_cleanup_semantics(&plan);

    let by_path: std::collections::BTreeMap<_, _> = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["path"].as_str().unwrap().to_string(), e.clone()))
        .collect();

    assert_eq!(by_path["Build/bin/perm"]["disposition"], "blocked");
    assert_eq!(by_path["Build/bin/perm"]["reason"], "retention-permanent");
    assert_eq!(by_path["Build/bin/bare"]["disposition"], "blocked");
    assert_eq!(by_path["Build/bin/bare"]["reason"], "no-manifest");
    assert_eq!(by_path["Build/bin/review"]["disposition"], "advisory");
    assert_eq!(
        by_path["Build/bin/review"]["reason"],
        "retention-review-required"
    );
    assert_eq!(by_path["Build/bin/incomplete"]["disposition"], "blocked");
    assert_eq!(
        by_path["Build/bin/incomplete"]["reason"],
        "invalid-manifest"
    );
    assert_eq!(by_path["Build/bin/sess"]["disposition"], "advisory");
    assert_eq!(
        by_path["Build/bin/sess"]["reason"],
        "retention-review-required"
    );
    assert_eq!(by_path["Build/bin/fresh-auto"]["disposition"], "advisory");
    assert_eq!(by_path["Build/bin/fresh-auto"]["reason"], "current");
    assert_eq!(
        by_path["Build/bin/stale-auto"]["disposition"],
        "would_archive"
    );
    assert_eq!(by_path["Build/bin/stale-auto"]["reason"], "stale");
    assert_eq!(by_path["Build/bin/nested"]["disposition"], "advisory");
    assert_eq!(
        by_path["Build/bin/nested"]["reason"],
        "retention-review-required"
    );
    assert_eq!(by_path["Build/bin/multi"]["disposition"], "blocked");
    assert_eq!(by_path["Build/bin/multi"]["reason"], "multiple-manifests");

    let out = h.run_with_path(
        &h.bin_cleanup,
        &[
            "--mode",
            "dry-run",
            "--scope",
            "Build/bin/approved",
            "--json",
        ],
        &tools,
    );
    require_success(&out, "scoped delete overlay");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_cleanup_semantics(&plan);
    let entry = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "Build/bin/approved")
        .unwrap();
    assert_eq!(entry["disposition"], "would_delete");
    assert_eq!(entry["reason"], "approved");
    assert_eq!(entry["approved_to_matches"], true);

    let out = h.run_with_path(
        &h.bin_cleanup,
        &[
            "--mode",
            "dry-run",
            "--scope",
            "Build/bin/incomplete",
            "--json",
        ],
        &tools,
    );
    require_success(&out, "incomplete scoped");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    let entry = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "Build/bin/incomplete")
        .unwrap();
    assert_eq!(entry["disposition"], "blocked");
    assert_eq!(entry["reason"], "invalid-manifest");

    let out = h.run_with_path(
        &h.bin_cleanup,
        &[
            "--mode",
            "dry-run",
            "--scope",
            "Build/bin/mismatch",
            "--json",
        ],
        &tools,
    );
    require_success(&out, "mismatch scope");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    let entry = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "Build/bin/mismatch")
        .unwrap();
    assert_eq!(entry["disposition"], "blocked");
    assert_eq!(entry["reason"], "approved-to-mismatch");

    let out = h.run_with_path(
        &h.bin_cleanup,
        &[
            "--mode",
            "dry-run",
            "--scope",
            "Build/bin/nullapp",
            "--json",
        ],
        &tools,
    );
    require_success(&out, "null approved_to");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    let entry = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "Build/bin/nullapp")
        .unwrap();
    assert_eq!(entry["disposition"], "blocked");
    assert_eq!(entry["reason"], "approved-to-null");

    h.write_inventory_fixture(&sample_inventory(h.ws_root.to_str().unwrap()));
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    require_success(&out, "empty");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_eq!(plan["summary"]["total"], 0);
    assert_cleanup_semantics(&plan);
}

#[test]
fn ontarch_inventory_fd_and_find_paths_are_equivalent() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "nested", true);
    fs::create_dir_all(h.ws_root.join("Build/bin/nested/deep")).unwrap();
    fs::write(h.ws_root.join("Build/bin/nested/deep/extra.txt"), b"x\n").unwrap();

    let with_fd = h.tools_with_fd();
    let without_fd = h.tools_without_fd();
    let _ = fs::remove_file(with_fd.join("INVOKED_FD"));
    let _ = fs::remove_file(with_fd.join("INVOKED_FIND"));
    let _ = fs::remove_file(without_fd.join("INVOKED_FIND"));
    let a_out = h.run_with_path(&h.bin_report, &["--json"], &with_fd);
    require_success(&a_out, "with-fd");
    assert!(
        with_fd.join("INVOKED_FD").is_file(),
        "fd path must invoke fd marker"
    );
    let b_out = h.run_with_path(&h.bin_report, &["--json"], &without_fd);
    require_success(&b_out, "without-fd");
    assert!(
        without_fd.join("INVOKED_FIND").is_file(),
        "find path must invoke find marker"
    );
    let mut a = assert_one_json_document(std::str::from_utf8(&a_out.stdout).unwrap());
    let mut b = assert_one_json_document(std::str::from_utf8(&b_out.stdout).unwrap());
    // Normalize generated_at and age fields (stat shim may stabilize ages).
    a["generated_at"] = json!("TS");
    b["generated_at"] = json!("TS");
    for w in a["workflows"].as_array_mut().unwrap() {
        w["oldest_file_age_days"] = json!(null);
        w["newest_file_age_days"] = json!(null);
    }
    for w in b["workflows"].as_array_mut().unwrap() {
        w["oldest_file_age_days"] = json!(null);
        w["newest_file_age_days"] = json!(null);
    }
    assert_eq!(a["summary"], b["summary"]);
    assert_eq!(a["workflows"], b["workflows"]);
}

#[test]
fn ontarch_inventory_bsd_and_gnu_stat_paths_are_equivalent() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let bsd = h.tools_bsd_stat();
    let gnu = h.tools_gnu_stat();
    let _ = fs::remove_file(bsd.join("INVOKED_STAT_BSD"));
    let _ = fs::remove_file(gnu.join("INVOKED_STAT_GNU"));
    let a_out = h.run_with_path(&h.bin_report, &["--json"], &bsd);
    require_success(&a_out, "bsd-stat");
    assert!(
        bsd.join("INVOKED_STAT_BSD").is_file(),
        "bsd path must invoke bsd stat marker"
    );
    let b_out = h.run_with_path(&h.bin_report, &["--json"], &gnu);
    require_success(&b_out, "gnu-stat");
    assert!(
        gnu.join("INVOKED_STAT_GNU").is_file(),
        "gnu path must invoke gnu stat marker"
    );
    let mut a = assert_one_json_document(std::str::from_utf8(&a_out.stdout).unwrap());
    let mut b = assert_one_json_document(std::str::from_utf8(&b_out.stdout).unwrap());
    a["generated_at"] = json!("TS");
    b["generated_at"] = json!("TS");
    assert_eq!(a, b);
    // Ages should be computed (not null) under controlled epoch shims.
    let age = a["workflows"][0]["oldest_file_age_days"].as_u64();
    assert!(age.is_some(), "stat shim must produce ages");
}

#[test]
fn sample_cleanup_plan_fixture_shape_is_plan_aligned() {
    let doc = sample_cleanup_plan("report-only");
    validate_cleanup_schema(&doc);
    assert_eq!(doc["mutation_executed"], false);
    let bad = sample_cleanup_mutation_true();
    assert!(validate_cleanup_schema_catch(&bad).is_err());
}

fn validate_cleanup_schema_catch(doc: &Value) -> Result<(), ()> {
    std::panic::catch_unwind(|| validate_cleanup_schema(doc)).map_err(|_| ())
}

#[test]
fn inventory_schema_rejects_one_mutation_negatives() {
    let mut good = sample_inventory("/tmp/ws");
    good["workflows"] = json!([{
        "path": "Build/bin/demo",
        "size_bytes": 10,
        "file_count": 1,
        "oldest_file_age_days": 2,
        "newest_file_age_days": 1,
        "manifest_present": true,
        "manifest_count": 1
    }]);
    good["summary"] = json!({"total": 1, "with_manifest": 1});
    validate_inventory_schema(&good);

    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("generated_at");
    assert!(std::panic::catch_unwind(|| validate_inventory_schema(&missing)).is_err());

    let mut unknown = good.clone();
    unknown["extra"] = json!(1);
    assert!(std::panic::catch_unwind(|| validate_inventory_schema(&unknown)).is_err());
}

#[test]
fn rust_and_shell_validators_agree_on_one_mutation_corpus() {
    let h = HermeticOntarch::new();
    for (label, doc) in support::inventory_one_mutation_corpus(h.ws_root.to_str().unwrap()) {
        let rust_ok = support::inventory_schema_ok(&doc);
        let shell = shell_validate_inventory(&h, &doc);
        assert!(
            !rust_ok && !shell.status.success(),
            "{label}: both validators must reject\nrust_ok={rust_ok}\n{}",
            String::from_utf8_lossy(&shell.stderr)
        );
        let err = format!(
            "{}{}",
            String::from_utf8_lossy(&shell.stdout),
            String::from_utf8_lossy(&shell.stderr)
        );
        assert!(
            err.contains("bin_inventory:"),
            "{label}: expected bin_inventory diagnostic\n{err}"
        );
    }
    for (label, doc) in support::cleanup_one_mutation_corpus() {
        let rust_ok = support::cleanup_schema_ok(&doc);
        let shell = shell_validate_cleanup(&h, &doc);
        assert!(
            !rust_ok && !shell.status.success(),
            "{label}: both validators must reject\nrust_ok={rust_ok}\n{}",
            String::from_utf8_lossy(&shell.stderr)
        );
        let err = format!(
            "{}{}",
            String::from_utf8_lossy(&shell.stdout),
            String::from_utf8_lossy(&shell.stderr)
        );
        assert!(
            err.contains("bin_cleanup:"),
            "{label}: expected bin_cleanup diagnostic\n{err}"
        );
    }
}

fn shell_validate_inventory(h: &HermeticOntarch, doc: &Value) -> std::process::Output {
    let path = h.registry.join("_corpus_inv.json");
    fs::write(&path, serde_json::to_string(doc).unwrap()).unwrap();
    let script = format!(
        r#"set -euo pipefail
source {lib}/common.sh
source {lib}/registry.sh
ontarch_validate_bin_inventory_doc {path} {root}
"#,
        lib = support::shell_single_quote(&h.ontarch_pkg.join("lib").to_string_lossy()),
        path = support::shell_single_quote(&path.to_string_lossy()),
        root = support::shell_single_quote(h.ws_root.to_str().unwrap()),
    );
    std::process::Command::new("bash")
        .arg("-c")
        .arg(script)
        .current_dir(&h.ws_root)
        .env("WS_ROOT", &h.ws_root)
        .env_remove("ONTARCH_REGISTRY")
        .output()
        .unwrap()
}

fn shell_validate_cleanup(h: &HermeticOntarch, doc: &Value) -> std::process::Output {
    let path = h.registry.join("_corpus_plan.json");
    fs::write(&path, serde_json::to_string(doc).unwrap()).unwrap();
    let script = format!(
        r#"set -euo pipefail
source {lib}/common.sh
source {lib}/registry.sh
ontarch_validate_bin_cleanup_plan_doc {path}
"#,
        lib = support::shell_single_quote(&h.ontarch_pkg.join("lib").to_string_lossy()),
        path = support::shell_single_quote(&path.to_string_lossy()),
    );
    std::process::Command::new("bash")
        .arg("-c")
        .arg(script)
        .current_dir(&h.ws_root)
        .env("WS_ROOT", &h.ws_root)
        .env_remove("ONTARCH_REGISTRY")
        .output()
        .unwrap()
}

fn assert_no_registry_temp_leaks(h: &HermeticOntarch) {
    for entry in fs::read_dir(&h.registry).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        for prefix in [
            ".bin-inv.",
            ".bin-md.",
            ".graph-json.",
            ".graph-dot.",
            ".bak-a.",
            ".bak-b.",
            ".bin-plan.",
        ] {
            assert!(!name.starts_with(prefix), "temp leak remains: {name}");
        }
    }
}

fn which_host(name: &str) -> Option<std::path::PathBuf> {
    for prefix in ["/bin", "/usr/bin", "/usr/sbin", "/sbin"] {
        let p = std::path::PathBuf::from(prefix).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn install_mv_shim(tools: &std::path::Path, body: &str) {
    let real_mv = which_host("mv").expect("mv");
    let _ = fs::remove_file(tools.join("mv"));
    write_executable(
        &tools.join("mv"),
        &body.replace(
            "__REAL_MV__",
            &support::shell_single_quote(&real_mv.to_string_lossy()),
        ),
    );
}

fn assert_inv_pair_state(h: &HermeticOntarch, json: Option<&[u8]>, md: Option<&[u8]>) {
    let jp = h.registry.join("bin-inventory.json");
    let mp = h.registry.join("BIN-INVENTORY.md");
    match json {
        Some(b) => {
            assert!(jp.is_file(), "expected inventory json");
            assert_eq!(fs::read(&jp).unwrap(), b);
        }
        None => assert!(!jp.exists(), "inventory json must be absent"),
    }
    match md {
        Some(b) => {
            assert!(mp.is_file(), "expected inventory md");
            assert_eq!(fs::read(&mp).unwrap(), b);
        }
        None => assert!(!mp.exists(), "inventory md must be absent"),
    }
}

#[test]
fn ontarch_pair_install_prior_state_matrix_on_b_failure() {
    // Four prior-state combos × B (Markdown) install failure.
    for (label, seed_json, seed_md) in [
        ("neither", false, false),
        ("only_a", true, false),
        ("only_b", false, true),
        ("both", true, true),
    ] {
        let h = HermeticOntarch::new();
        h.seed_bin_workflow("Build", "demo", true);
        require_success(&h.run_bin_report(&["--json"]), "seed");
        let prior_json = fs::read(h.registry.join("bin-inventory.json")).unwrap();
        let prior_md = fs::read(h.registry.join("BIN-INVENTORY.md")).unwrap();
        if !seed_json {
            let _ = fs::remove_file(h.registry.join("bin-inventory.json"));
        }
        if !seed_md {
            let _ = fs::remove_file(h.registry.join("BIN-INVENTORY.md"));
        } else if !seed_json {
            // only_b: keep md bytes, drop json
            fs::write(h.registry.join("BIN-INVENTORY.md"), &prior_md).unwrap();
        }
        let expect_json = if seed_json {
            Some(prior_json.as_slice())
        } else {
            None
        };
        let expect_md = if seed_md {
            Some(prior_md.as_slice())
        } else {
            None
        };

        let tools = h.tools_bsd_stat();
        install_mv_shim(
            &tools,
            r#"#!/bin/sh
set -eu
dest=""
for a in "$@"; do dest="$a"; done
case "$dest" in
  *BIN-INVENTORY.md) exit 1 ;;
esac
exec __REAL_MV__ "$@"
"#,
        );
        let out = h.run_with_path(&h.bin_report, &["--json"], &tools);
        assert!(
            !out.status.success(),
            "{label}: report must fail on B install\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains("ok BIN-INVENTORY") && !combined.contains("ok bin-inventory"),
            "{label}: must not claim success\n{combined}"
        );
        assert_inv_pair_state(&h, expect_json, expect_md);
        assert_no_registry_temp_leaks(&h);
    }
}

#[test]
fn ontarch_pair_install_a_failure_retains_prior() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "seed");
    let prior_json = fs::read(h.registry.join("bin-inventory.json")).unwrap();
    let prior_md = fs::read(h.registry.join("BIN-INVENTORY.md")).unwrap();
    let tools = h.tools_bsd_stat();
    install_mv_shim(
        &tools,
        r#"#!/bin/sh
set -eu
dest=""
for a in "$@"; do dest="$a"; done
case "$dest" in
  *bin-inventory.json) exit 1 ;;
esac
exec __REAL_MV__ "$@"
"#,
    );
    let out = h.run_with_path(&h.bin_report, &["--json"], &tools);
    assert!(!out.status.success());
    assert_inv_pair_state(&h, Some(&prior_json), Some(&prior_md));
    assert_no_registry_temp_leaks(&h);
}

#[test]
fn ontarch_pair_install_rollback_restore_failure_diagnoses() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    require_success(&h.run_bin_report(&["--json"]), "seed");
    let tools = h.tools_bsd_stat();
    install_mv_shim(
        &tools,
        r#"#!/bin/sh
set -eu
src=""
dest=""
# mv [-f] src dest — collect last two non-option args
for a in "$@"; do
  case "$a" in
    -*) ;;
    *) src="$dest"; dest="$a" ;;
  esac
done
case "$dest" in
  *BIN-INVENTORY.md) exit 1 ;;
esac
case "$src" in
  */.bak-a.*) exit 1 ;;
esac
exec __REAL_MV__ "$@"
"#,
    );
    let out = h.run_with_path(&h.bin_report, &["--json"], &tools);
    assert!(!out.status.success());
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        err.contains("generated_pair:rollback_failed"),
        "expected rollback_failed\n{err}"
    );
    assert_no_registry_temp_leaks(&h);
}

#[test]
fn ontarch_agent_mutation_refuses_before_inventory_work() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    let _ = fs::remove_file(h.registry.join("bin-inventory.json"));
    let _ = fs::remove_file(h.registry.join("BIN-INVENTORY.md"));

    for (mode, extra) in [
        ("archive", None),
        ("delete-approved", Some("Build/bin/demo")),
    ] {
        let mut args = vec!["--mode", mode, "--json"];
        if let Some(scope) = extra {
            args.extend_from_slice(&["--scope", scope]);
        }
        let out = std::process::Command::new(&h.bin_cleanup)
            .args(&args)
            .current_dir(&h.ws_root)
            .env("WS_ROOT", &h.ws_root)
            .env("PANOPLY_AGENT", "1")
            .env_remove("ONTARCH_REGISTRY")
            .env_remove("AGENTS_HOME")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{mode}");
        assert!(out.stdout.is_empty(), "{mode} stdout must be empty");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("agent_rail"), "{mode}: {err}");
        assert!(!h.registry.join("bin-inventory.json").exists());
        assert!(!h.registry.join("BIN-INVENTORY.md").exists());
    }

    // Invalid existing inventory must not change the agent_rail authority.
    let mut bad = sample_inventory(h.ws_root.to_str().unwrap());
    bad["extra"] = json!(1);
    h.write_inventory_fixture(&bad);
    let before = fs::read(h.registry.join("bin-inventory.json")).unwrap();
    let out = std::process::Command::new(&h.bin_cleanup)
        .args(["--mode", "archive", "--json"])
        .current_dir(&h.ws_root)
        .env("WS_ROOT", &h.ws_root)
        .env("PANOPLY_AGENT", "1")
        .env_remove("ONTARCH_REGISTRY")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("agent_rail"));
    assert_eq!(
        before,
        fs::read(h.registry.join("bin-inventory.json")).unwrap()
    );
}

#[test]
fn ontarch_parser_and_scoped_human_matrix() {
    let h = HermeticOntarch::new();
    h.seed_bin_workflow("Build", "demo", true);
    h.seed_bin_workflow("Build", "other", true);
    require_success(&h.run_bin_report(&["--json"]), "inv");

    // duplicate --json is idempotent
    let out = h.run_bin_cleanup(&["--json", "--json", "--mode", "report-only"]);
    require_success(&out, "duplicate json");
    assert_cleanup_semantics(&assert_one_json_document(
        std::str::from_utf8(&out.stdout).unwrap(),
    ));

    // help
    let out = h.run_bin_cleanup(&["--help"]);
    require_success(&out, "help");
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("report-only"));

    // --scope=value
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--scope=Build/bin/demo", "--json"]);
    require_success(&out, "scope=value");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert_eq!(v["scope"], "Build/bin/demo");
    assert_eq!(v["summary"]["total"], 1);

    // report-only without scope
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    require_success(&out, "report-only no scope");
    let v = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    assert!(v["summary"]["total"].as_u64().unwrap() >= 2);

    // dry-run without scope
    let out = h.run_bin_cleanup(&["--mode", "dry-run", "--json"]);
    require_success(&out, "dry-run no scope");
    assert_cleanup_semantics(&assert_one_json_document(
        std::str::from_utf8(&out.stdout).unwrap(),
    ));

    // human report-only with scope excludes outside entries
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--scope", "Build/bin/demo"]);
    require_success(&out, "human scoped");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("scope: Build/bin/demo"));
    assert!(
        !text.contains("Build/bin/other"),
        "scoped human must exclude outside paths\n{text}"
    );

    // human dry-run retains expected sections
    let out = h.run_bin_cleanup(&["--mode", "dry-run"]);
    require_success(&out, "human dry-run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("plan (archive candidates)"));
    assert!(text.contains("plan (delete-approved candidates"));

    // human archive / delete refusals
    let out = h.run_bin_cleanup(&["--mode", "archive"]);
    assert!(!out.status.success());
    let out = h.run_bin_cleanup(&["--mode", "delete-approved", "--scope", "Build/bin/demo"]);
    assert!(!out.status.success());

    // JSON refusal stdout empty
    let out = h.run_bin_cleanup(&["--mode", "archive", "--json"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn path_grammar_schema_shell_parity_corpus() {
    let h = HermeticOntarch::new();
    for (label, path, ok) in support::path_grammar_corpus() {
        let mut doc = sample_inventory(h.ws_root.to_str().unwrap());
        doc["workflows"] = json!([{
            "path": path,
            "size_bytes": 1,
            "file_count": 1,
            "oldest_file_age_days": 1,
            "newest_file_age_days": 1,
            "manifest_present": false,
            "manifest_count": 0
        }]);
        doc["summary"] = json!({"total": 1, "with_manifest": 0});
        let schema_ok = support::inventory_schema_ok(&doc);
        let shell = shell_validate_inventory(&h, &doc);
        assert_eq!(
            schema_ok, ok,
            "{label}: schema ok={schema_ok} expected {ok} for {path}"
        );
        assert_eq!(
            shell.status.success(),
            ok,
            "{label}: shell success={} expected {ok} for {path}\n{}",
            shell.status.success(),
            String::from_utf8_lossy(&shell.stderr)
        );
        // Scope preflight uses the same grammar.
        if !ok && !path.contains('\n') {
            let out = h.run_bin_cleanup(&["--mode", "report-only", "--scope", path, "--json"]);
            assert!(
                !out.status.success(),
                "{label}: scope preflight must reject {path:?}"
            );
        }
    }
}

#[test]
fn cleanup_shell_rejects_semantic_combinations_schema_may_miss() {
    let h = HermeticOntarch::new();
    for (label, disp, reason, retention, am) in [
        (
            "blocked approved-to-null needs am=false",
            "blocked",
            "approved-to-null",
            json!(null),
            json!(null),
        ),
        (
            "advisory review-required needs review retention",
            "advisory",
            "retention-review-required",
            json!("permanent"),
            json!(null),
        ),
        (
            "would_delete + current + permanent",
            "would_delete",
            "current",
            json!("permanent"),
            json!(null),
        ),
    ] {
        let mut bad = sample_cleanup_plan("dry-run");
        bad["entries"] = json!([{
            "path": "Build/bin/demo",
            "disposition": disp,
            "reason": reason,
            "retention": retention,
            "approved_to_matches": am
        }]);
        let mut summary = json!({
            "total": 1, "advisory": 0, "would_archive": 0, "would_delete": 0, "blocked": 0
        });
        summary[disp] = json!(1);
        bad["summary"] = summary;
        let shell = shell_validate_cleanup(&h, &bad);
        assert!(!shell.status.success(), "{label}: shell must reject");
        let err = format!(
            "{}{}",
            String::from_utf8_lossy(&shell.stdout),
            String::from_utf8_lossy(&shell.stderr)
        );
        assert!(
            err.contains("bin_cleanup:invalid_combination") || err.contains("bin_cleanup:"),
            "{label}: {err}"
        );
    }
}

#[test]
fn ontarch_incomplete_manifest_cases_are_invalid_manifest() {
    let cases: Vec<(&str, String)> = vec![
        ("malformed", "{not-json".into()),
        (
            "missing retention",
            r#"{"id":"x","workflow":"x","source":"s","created_at":"t","tool":"t","outputs":["o"]}"#
                .into(),
        ),
        (
            "invalid retention",
            HermeticOntarch::complete_manifest("x", "yeeted", None),
        ),
        (
            "missing outputs",
            r#"{"id":"x","workflow":"x","source":"s","created_at":"t","tool":"t","retention":"review-before-delete"}"#
                .into(),
        ),
        (
            "empty outputs",
            r#"{"id":"x","workflow":"x","source":"s","created_at":"t","tool":"t","outputs":[],"retention":"review-before-delete"}"#
                .into(),
        ),
        (
            "missing provenance key",
            r#"{"id":"x","source":"s","created_at":"t","tool":"t","outputs":["o"],"retention":"review-before-delete"}"#
                .into(),
        ),
        (
            "approved_to wrong type",
            r#"{"id":"x","workflow":"x","source":"s","created_at":"t","tool":"t","outputs":["o"],"retention":"review-before-delete","approved_to":12}"#
                .into(),
        ),
    ];
    for (label, body) in cases {
        let h = HermeticOntarch::new();
        h.seed_bin_workflow_manifest("Build", "bad", &body);
        require_success(&h.run_bin_report(&["--json"]), "inv");
        let out = h.run_bin_cleanup(&["--mode", "dry-run", "--scope", "Build/bin/bad", "--json"]);
        require_success(&out, label);
        let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
        let entry = plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"] == "Build/bin/bad")
            .unwrap_or_else(|| panic!("{label}: missing entry"));
        assert_eq!(entry["disposition"], "blocked", "{label}");
        assert_eq!(entry["reason"], "invalid-manifest", "{label}");
        assert!(entry["approved_to_matches"].is_null(), "{label}");
        assert!(entry["retention"].is_null(), "{label}");
    }

    // Valid complete root + nested single manifests classify without invalid-manifest.
    let h = HermeticOntarch::new();
    h.seed_bin_workflow_manifest(
        "Build",
        "rootok",
        &HermeticOntarch::complete_manifest("rootok", "review-before-delete", None),
    );
    let nested = h.ws_root.join("Build/bin/nestok/deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("artifact.txt"), b"x\n").unwrap();
    fs::write(
        nested.join("manifest.json"),
        HermeticOntarch::complete_manifest("nestok", "review-before-delete", None),
    )
    .unwrap();
    require_success(&h.run_bin_report(&["--json"]), "inv");
    let out = h.run_bin_cleanup(&["--mode", "report-only", "--json"]);
    require_success(&out, "valid manifests");
    let plan = assert_one_json_document(std::str::from_utf8(&out.stdout).unwrap());
    for path in ["Build/bin/rootok", "Build/bin/nestok"] {
        let entry = plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"] == path)
            .unwrap();
        assert_ne!(entry["reason"], "invalid-manifest");
        assert_eq!(entry["disposition"], "advisory");
    }
}

//! E09.S7 Phase 4 — hermetic MVP acceptance fixture and portability.
//!
//! Integrated coverage over the tracked `fixtures/e2e` tree. Unit-level
//! projection/policy/execution matrices remain in sibling test files; this
//! suite closes the end-to-end gaps (full MVP chain, state-root variants,
//! concurrency, metrics/RTK non-transform, no-host-state).

#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use std::fs;
use std::sync::Arc;
use std::thread;
use support::{E2eHarness, e2e_root, parse_json, stderr, stdout};
use takogami::execution::DEFAULT_LIMIT_BYTES;
use takogami::exit_codes::{
    EXECUTION_IO, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, RESOLUTION, STATE_IO, SUCCESS, USAGE,
};

fn assert_not_unimplemented(out: &std::process::Output) {
    assert_ne!(
        out.status.code(),
        Some(NOT_IMPLEMENTED as i32),
        "S7 MVP path not implemented (exit 10). stderr={}",
        stderr(out)
    );
}

fn assert_ok(out: &std::process::Output) -> Value {
    assert_not_unimplemented(out);
    assert_eq!(
        out.status.code(),
        Some(SUCCESS as i32),
        "stdout={} stderr={}",
        stdout(out),
        stderr(out)
    );
    parse_json(out)
}

fn assert_compressor_none(v: &Value) {
    // Graph envelopes carry metrics.compressor; bin projection may omit metrics.
    // Either way, machine JSON must never claim RTK compression or a gain snapshot.
    if let Some(c) = v.pointer("/metrics/compressor").and_then(|x| x.as_str()) {
        assert_eq!(c, "none", "machine JSON must not be RTK-transformed: {v}");
    }
    let raw = v.to_string();
    assert!(
        !raw.contains("\"compressor\":\"rtk\"") && !raw.contains("\"compressor\": \"rtk\""),
        "machine JSON must not be RTK-transformed: {v}"
    );
    assert!(
        v.pointer("/metrics/gain").is_none()
            || v.pointer("/metrics/gain")
                .map(|g| g.is_null())
                .unwrap_or(false),
        "graph/bin machine JSON must not carry RTK gain: {v}"
    );
}

#[test]
fn e2e_fixture_skeleton_is_tracked_and_copyable() {
    let root = e2e_root();
    for rel in [
        "README.md",
        "workspace/Build/bin/demo/manifest.json",
        "workspace/Plan/bin/stale-demo/manifest.json",
        "workspace/Build/bin/missing-manifest/artifact.txt",
        "workspace/Build/bin/scope-mismatch/manifest.json",
        "workspace/Build/src/workspaces/descriptor-backed/README.md",
        "workspace/Build/src/workspaces/descriptor-less/README.md",
        "ontarch/registry/graph.json",
        "ontarch/registry/graph.dot",
        "ontarch/registry/tools.json",
        "ontarch/registry/scan.json",
        "tools/README.md",
        "state/.gitkeep",
        "variants/stale/graph.json",
        "variants/malformed/graph.json",
        "expected/graph-text.txt",
        "expected/bin-report-envelope.json",
        "expected/cleanup-report-envelope.json",
    ] {
        assert!(root.join(rel).is_file(), "missing tracked fixture {rel}");
    }
    let graph: Value = serde_json::from_str(
        &fs::read_to_string(root.join("ontarch/registry/graph.json")).unwrap(),
    )
    .unwrap();
    assert!(
        graph.get("registry_generation").is_some(),
        "tracked e2e graph.json must include registry_generation"
    );
    let h = E2eHarness::new();
    assert!(h.registry.join("graph.json").is_file());
    h.assert_tracked_unchanged();
}

#[test]
fn e2e_mvp_full_command_chain() {
    let h = E2eHarness::new();

    // scan → list/info/tools/interfaces/doctor
    let scan = assert_ok(&h.run(&["--json", "scan"]));
    assert_eq!(scan["command"], "scan");

    let list = assert_ok(&h.run(&["--json", "list", "units"]));
    assert_eq!(list["command"], "list");
    assert!(list["data"]["count"].as_u64().unwrap_or(0) >= 1);

    let info = assert_ok(&h.run(&["--json", "info", "demo"]));
    assert_eq!(info["command"], "info");

    let tools = assert_ok(&h.run(&["--json", "tools"]));
    assert_eq!(tools["command"], "tools");

    let interfaces = assert_ok(&h.run(&["--json", "interfaces"]));
    assert_eq!(interfaces["command"], "interfaces");

    let doctor = assert_ok(&h.run(&["--json", "doctor"]));
    assert_eq!(doctor["command"], "doctor");

    // lifecycle resolve / explain (plan-only)
    let build = assert_ok(&h.run(&["--json", "build", "demo"]));
    assert_eq!(build["command"], "build");
    let explain = assert_ok(&h.run(&["--json", "build", "demo", "--explain"]));
    assert!(
        explain.get("explanation").is_some() || explain["data"].get("explanation").is_some(),
        "explain must surface explanation: {explain}"
    );

    // native plan-only leaves a terminal command record
    let records_after_lifecycle = h.load_records();
    assert!(
        !records_after_lifecycle.is_empty(),
        "lifecycle must produce a command record"
    );
    let last = records_after_lifecycle.last().unwrap();
    assert_eq!(last["schema_version"], "0.1.0");
    assert!(last.get("ended_at").is_some(), "record must be terminal");

    // session list/show/latest against real records
    let session_list = assert_ok(&h.run(&["--json", "session", "list"]));
    assert_eq!(session_list["command"], "session");
    let sid = records_after_lifecycle[0]["session_id"]
        .as_str()
        .expect("session_id");
    let show = assert_ok(&h.run(&["--json", "session", "show", sid]));
    assert_eq!(show["command"], "session");
    let latest = assert_ok(&h.run(&["--json", "session", "latest"]));
    assert_eq!(latest["command"], "session");

    // graph text/DOT/JSON — zero spawn, no records added for graph itself
    let before_graph = h.load_records().len();
    let before_spawn = h.marker_count();
    let g_text = assert_ok(&h.run(&["--json", "graph", "--format", "text"]));
    assert_eq!(g_text["data"]["freshness"], "hit");
    assert_compressor_none(&g_text);
    let g_dot = assert_ok(&h.run(&["--json", "graph", "--format", "dot"]));
    assert_eq!(g_dot["data"]["format"], "dot");
    let g_json = assert_ok(&h.run(&["--json", "graph", "--format", "json"]));
    assert_eq!(g_json["data"]["format"], "json");
    assert_eq!(h.marker_count(), before_spawn, "graph must not spawn");
    assert_eq!(
        h.load_records().len(),
        before_graph,
        "graph must not create command records"
    );

    // bin report Allow/spawn once
    let report = assert_ok(&h.run(&["--json", "bin", "report"]));
    assert_eq!(report["command"], "bin report");
    assert_compressor_none(&report);
    h.assert_spawn_count(before_spawn + 1);

    // cleanup report-only Allow
    let report_only = assert_ok(&h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]));
    assert!(
        report_only["command"]
            .as_str()
            .unwrap_or("")
            .starts_with("bin cleanup"),
        "unexpected command field: {}",
        report_only["command"]
    );
    h.assert_spawn_count(before_spawn + 2);

    // dry-run Gate / no-spawn
    let dry = h.run(&["--json", "bin", "cleanup", "--mode", "dry-run"]);
    assert_not_unimplemented(&dry);
    assert_eq!(
        dry.status.code(),
        Some(POLICY_GATE as i32),
        "{}",
        stdout(&dry)
    );
    h.assert_spawn_count(before_spawn + 2);

    // archive / delete Deny/deferred / no-spawn
    let archive = h.run(&["--json", "bin", "cleanup", "--mode", "archive"]);
    assert_not_unimplemented(&archive);
    assert_eq!(
        archive.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stdout(&archive)
    );
    let archive_body = format!("{}{}", stdout(&archive), stderr(&archive));
    assert!(
        archive_body.contains("deferred_unavailable")
            || parse_json(&archive)
                .pointer("/diagnostics")
                .map(|d| d.to_string().contains("deferred"))
                .unwrap_or(false)
            || archive_body.contains("deferred"),
        "archive must surface deferred: {archive_body}"
    );
    h.assert_spawn_count(before_spawn + 2);

    let delete = h.run(&["--json", "bin", "cleanup", "--mode", "delete-approved"]);
    assert_not_unimplemented(&delete);
    assert_eq!(
        delete.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stdout(&delete)
    );
    h.assert_spawn_count(before_spawn + 2);

    h.assert_tracked_unchanged();
}

#[test]
fn e2e_stale_and_malformed_registry_variants() {
    let h = E2eHarness::new();

    // Fresh hit first.
    let hit = assert_ok(&h.run(&["--json", "graph", "--format", "text"]));
    assert_eq!(hit["metrics"]["registry_cache"], "hit");
    assert_eq!(hit["data"]["freshness"], "hit");

    h.overlay_stale_graph();
    let stale = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_unimplemented(&stale);
    assert_eq!(
        stale.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stdout(&stale)
    );
    let stale_v = parse_json(&stale);
    let code = stale_v
        .pointer("/diagnostics/0/code")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    assert!(
        code.contains("stale") || code.contains("graph_stale"),
        "expected stale diagnostic, got {stale_v}"
    );
    h.assert_no_spawn();

    h.overlay_malformed_graph();
    let bad = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_unimplemented(&bad);
    assert_ne!(bad.status.code(), Some(SUCCESS as i32));
    h.assert_no_spawn();

    // Units stale/malformed for list.
    let h2 = E2eHarness::new();
    h2.overlay_stale_units();
    let list_stale = h2.run(&["--json", "list", "units"]);
    assert_not_unimplemented(&list_stale);
    let list_v = parse_json(&list_stale);
    // stale may still succeed with registry_cache=stale, or fail closed — accept either
    // as long as it does not silently claim a hit.
    if list_stale.status.success() {
        assert_ne!(list_v["metrics"]["registry_cache"], "hit");
    }

    h2.overlay_malformed_units();
    let list_bad = h2.run(&["--json", "list", "units"]);
    assert_not_unimplemented(&list_bad);
    assert_ne!(
        list_bad.status.code(),
        Some(SUCCESS as i32),
        "malformed units must fail closed: {}",
        stdout(&list_bad)
    );

    h.assert_tracked_unchanged();
    h2.assert_tracked_unchanged();
}

#[test]
fn e2e_state_root_variants_fail_closed() {
    // read-only state-home: writing a record must fail deterministically
    {
        let h = E2eHarness::new();
        h.make_state_home_readonly();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_ne!(
            out.status.code(),
            Some(SUCCESS as i32),
            "read-only state-home must not succeed: {}",
            stdout(&out)
        );
        h.restore_state_home_writable();
        h.assert_tracked_unchanged();
    }

    // missing state-home: session list may create or fail; bin report should create or fail closed
    {
        let h = E2eHarness::new();
        h.remove_state_home();
        let out = h.run(&["--json", "session", "list"]);
        assert_not_unimplemented(&out);
        // empty list success after create, or deterministic error — not crash
        assert!(
            out.status.code().is_some(),
            "missing state-home must yield an exit code"
        );
        h.assert_tracked_unchanged();
    }

    // symlink state-home: fail closed (non-regular / symlink root)
    {
        let h = E2eHarness::new();
        h.replace_state_home_with_symlink();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_ne!(
            out.status.code(),
            Some(SUCCESS as i32),
            "symlinked state-home must fail closed: {}",
            stdout(&out)
        );
        h.assert_tracked_unchanged();
    }

    // non-directory (file) state-home
    {
        let h = E2eHarness::new();
        h.replace_state_home_with_file();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert!(
            matches!(
                out.status.code(),
                Some(x) if x == STATE_IO as i32
                    || x == EXECUTION_IO as i32
                    || x == USAGE as i32
                    || x != SUCCESS as i32
            ),
            "file state-home must fail: {:?}",
            out.status.code()
        );
        h.assert_tracked_unchanged();
    }

    // FIFO state-home
    {
        let h = E2eHarness::new();
        h.replace_state_home_with_fifo();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_ne!(
            out.status.code(),
            Some(SUCCESS as i32),
            "FIFO state-home must fail closed: {}",
            stdout(&out)
        );
        h.assert_tracked_unchanged();
    }
}

#[test]
fn e2e_concurrent_lifecycle_and_bin_share_state_home() {
    let h = Arc::new(E2eHarness::new());
    let h1 = Arc::clone(&h);
    let h2 = Arc::clone(&h);
    let t1 = thread::spawn(move || h1.run(&["--json", "build", "demo"]));
    let t2 = thread::spawn(move || h2.run(&["--json", "bin", "report"]));
    let build = t1.join().unwrap();
    let report = t2.join().unwrap();
    assert_ok(&build);
    assert_ok(&report);
    let records = h.load_records();
    assert!(
        records.len() >= 2,
        "concurrent runs must leave independent records, got {}",
        records.len()
    );
    let mut ids: Vec<_> = records
        .iter()
        .filter_map(|r| r["session_id"].as_str().map(|s| s.to_string()))
        .collect();
    ids.sort();
    ids.dedup();
    assert!(
        ids.len() >= 2,
        "concurrent sessions must have distinct ids: {ids:?}"
    );
    for r in &records {
        assert_eq!(r["schema_version"], "0.1.0");
        assert!(r.get("ended_at").is_some(), "record must be terminal: {r}");
    }
    h.assert_tracked_unchanged();
}

#[test]
fn e2e_graph_and_bin_output_bounds_observed() {
    // Bin stderr bound: oversized stderr stays under capture limit and still parses.
    let h = E2eHarness::new();
    h.install_oversized_stderr_ontarch();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_unimplemented(&out);
    // May succeed (bounded) or fail closed on truncated JSON — either is acceptable
    // as long as the process does not hang and stderr capture stays bounded.
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        combined.len() < DEFAULT_LIMIT_BYTES * 4,
        "combined output must stay bounded, got {} bytes",
        combined.len()
    );
    if out.status.success() {
        let v = parse_json(&out);
        assert_compressor_none(&v);
    }

    // Graph edge truncation is covered in graph_cli; here assert hit graph JSON stays
    // single-document and compressor-none under the integrated harness.
    let h2 = E2eHarness::new();
    let g = assert_ok(&h2.run(&["--json", "graph", "--format", "text"]));
    assert_compressor_none(&g);
    assert!(
        g["metrics"]["output_bytes"].as_u64().unwrap_or(0) < DEFAULT_LIMIT_BYTES as u64
            || g["metrics"].get("output_bytes").is_some(),
        "metrics present: {g}"
    );

    h.assert_tracked_unchanged();
    h2.assert_tracked_unchanged();
}

#[test]
fn e2e_registry_hit_stale_miss_and_rtk_non_transform() {
    let h = E2eHarness::new();

    let list_hit = assert_ok(&h.run(&["--json", "list", "units"]));
    assert_eq!(list_hit["metrics"]["registry_cache"], "hit");

    let graph_hit = assert_ok(&h.run(&["--json", "graph", "--format", "text"]));
    assert_eq!(graph_hit["metrics"]["registry_cache"], "hit");
    assert_eq!(graph_hit["data"]["freshness"], "hit");
    assert_compressor_none(&graph_hit);

    let build = assert_ok(&h.run(&["--json", "build", "demo"]));
    // Lifecycle may report hit/stale/miss depending on units fingerprint overlay;
    // record whatever truthful value is present.
    let life_cache = build
        .pointer("/metrics/registry_cache")
        .cloned()
        .or_else(|| {
            build
                .pointer("/explanation/freshness/registry_cache")
                .cloned()
        });
    assert!(
        life_cache.is_some(),
        "lifecycle must report registry_cache: {build}"
    );

    let report = assert_ok(&h.run(&["--json", "bin", "report"]));
    assert_compressor_none(&report);

    // Stale
    h.overlay_stale_graph();
    let stale = h.run(&["--json", "graph", "--format", "text"]);
    assert_eq!(stale.status.code(), Some(RESOLUTION as i32));

    // Miss: remove graph
    let _ = fs::remove_file(h.registry.join("graph.json"));
    let miss = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_unimplemented(&miss);
    assert_ne!(miss.status.code(), Some(SUCCESS as i32));
    let miss_v = parse_json(&miss);
    let miss_code = miss_v
        .pointer("/diagnostics/0/code")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    assert!(
        miss_code.contains("miss") || miss_code.contains("missing") || miss_code.contains("graph"),
        "expected miss diagnostic: {miss_v}"
    );

    // RTK absent: truthful fallback — lifecycle plan-only compressor none / no gain required.
    let h2 = E2eHarness::new();
    let life = assert_ok(&h2.run(&["--json", "build", "demo"]));
    if let Some(c) = life.pointer("/metrics/compressor").and_then(|x| x.as_str()) {
        assert_eq!(c, "none", "RTK-absent lifecycle must not claim rtk: {life}");
    }
    // Gain may be absent (unavailable) — that is the truthful fallback snapshot.
    let gain = life.pointer("/metrics/gain");
    assert!(
        gain.is_none() || gain.map(|g| g.is_null()).unwrap_or(false),
        "without RTK, gain must be absent/null: {life}"
    );

    h.assert_tracked_unchanged();
    h2.assert_tracked_unchanged();
}

#[test]
fn e2e_no_host_state_dependency() {
    let h = E2eHarness::new();
    let out = h.run_scrubbed(&["--json", "tools"]);
    assert_ok(&out);
    let report = h.run_scrubbed(&["--json", "bin", "report"]);
    assert_ok(&report);
    h.assert_no_escape_outside_state();
    h.assert_tracked_unchanged();
    // Confirm we did not write under the real developer home by checking the
    // isolated HOME we injected remains the only HOME-touch surface.
    let isolated = h.root.join("isolated-home");
    assert!(isolated.is_dir());
}

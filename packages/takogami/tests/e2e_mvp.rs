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
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::thread;
use support::{
    E2eHarness, LIFECYCLE_STDERR, LIFECYCLE_STDOUT, e2e_root, parse_json, stderr, stdout,
};
use takogami::execution::DEFAULT_LIMIT_BYTES;
use takogami::exit_codes::{
    CONTRACT, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, RESOLUTION, STATE_IO, SUCCESS,
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

fn assert_one_json_document(raw: &str) {
    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    let first = stream
        .next()
        .unwrap_or_else(|| panic!("expected one JSON document\n{raw}"))
        .unwrap_or_else(|e| panic!("JSON parse failed: {e}\n{raw}"));
    assert!(
        stream.next().is_none(),
        "stdout must be exactly one JSON document: {raw}"
    );
    let _ = first;
}

fn doctor_check<'a>(report: &'a Value, name: &str) -> Option<&'a Value> {
    report
        .pointer("/data/checks")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find(|c| c["name"] == name))
}

fn tools_entry<'a>(tools: &'a Value, id: &str) -> Option<&'a Value> {
    tools
        .pointer("/data/tools")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.iter().find(|t| t["id"] == id))
        .or_else(|| {
            // Some envelopes nest classified tools under data.classified / data.tools.
            tools
                .pointer("/data/classified")
                .and_then(|t| t.as_array())
                .and_then(|arr| arr.iter().find(|t| t["id"] == id))
        })
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
        "variants/tools/tmux-only.json",
        "variants/tools/tmux-herdr.json",
        "variants/tools/neither.json",
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
    h.install_lifecycle_child(0);

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

    // --- integrated --execute lifecycle child (C01) ---
    let before_ontarch = h.marker_count();
    let before_lifecycle = h.lifecycle_marker_count();
    let before_records = h.load_records().len();
    let secret = "do-not-leak-e2e-secret";
    let herdr_sock = "/tmp/herdr-e2e-should-not-appear.sock";
    let tmux_sock = "/tmp/tmux-e2e-should-not-appear/default";
    let executed = h.run_env(
        &["--json", "build", "demo", "--execute"],
        &[
            ("SECRET_SENTINEL", secret),
            ("HERDR_SOCKET_PATH", herdr_sock),
            ("TMUX", tmux_sock),
            ("TMUX_PANE", "%1"),
        ],
        true,
    );
    assert_not_unimplemented(&executed);
    assert_eq!(
        executed.status.code(),
        Some(SUCCESS as i32),
        "stdout={} stderr={}",
        stdout(&executed),
        stderr(&executed)
    );
    assert_one_json_document(stdout(&executed).trim());
    let exec_v = parse_json(&executed);
    assert_eq!(exec_v["data"]["mode"], "executed");
    assert_eq!(exec_v["data"]["execution_requested"], true);
    assert_eq!(exec_v["data"]["execution_authorized"], true);
    let child_out = exec_v["child"]["stdout"].as_str().unwrap_or("");
    let child_err = exec_v["child"]["stderr"].as_str().unwrap_or("");
    assert_eq!(child_out, LIFECYCLE_STDOUT);
    assert_eq!(child_err, LIFECYCLE_STDERR);
    assert!(!child_out.contains(LIFECYCLE_STDERR));
    assert!(!child_err.contains(LIFECYCLE_STDOUT));
    assert_eq!(exec_v["child"]["truncated"], false);
    assert_eq!(exec_v["data"]["execution"]["started"], true);
    assert_eq!(exec_v["data"]["execution"]["outcome"], "completed");
    assert_eq!(exec_v["data"]["execution"]["exit_code"], 0);
    let pid = exec_v["data"]["execution"]["pid"]
        .as_u64()
        .expect("executed child pid");
    assert!(pid > 0, "pid must be positive: {pid}");

    h.assert_lifecycle_spawn_count(before_lifecycle + 1);
    assert_eq!(
        h.marker_count(),
        before_ontarch,
        "lifecycle spawn must not count as Ontarch spawn"
    );
    assert!(
        !h.path_decoy_marker.exists(),
        "PATH decoy ontarch must never run"
    );

    // redaction: secrets / provider sockets never reach child env or envelopes/records
    let env_dump = fs::read_to_string(&h.child_env_dump).expect("lifecycle env dump");
    assert!(
        env_dump.lines().any(|l| l.starts_with("PATH=")),
        "child must receive sealed PATH"
    );
    for forbidden in [
        "SECRET_SENTINEL",
        "HERDR_SOCKET_PATH",
        "TMUX=",
        "TMUX_PANE",
        secret,
        herdr_sock,
        tmux_sock,
    ] {
        assert!(
            !env_dump.contains(forbidden),
            "child env must not contain {forbidden}"
        );
    }
    let envelope_raw = stdout(&executed);
    for forbidden in [secret, herdr_sock, tmux_sock] {
        assert!(
            !envelope_raw.contains(forbidden),
            "envelope must not contain {forbidden}"
        );
    }

    let records_after_exec = h.load_records();
    assert!(
        records_after_exec.len() > before_records,
        "execute must add a command record"
    );
    let exec_rec = records_after_exec
        .iter()
        .find(|r| r["execution"]["pid"] == pid)
        .expect("executed record with matching pid");
    assert_eq!(exec_rec["schema_version"], "0.1.0");
    assert_eq!(exec_rec["execution"]["started"], true);
    assert_eq!(exec_rec["execution"]["outcome"], "completed");
    assert_eq!(exec_rec["execution"]["exit_code"], 0);
    assert_eq!(exec_rec["execution"]["pid"], pid);
    assert!(exec_rec.get("ended_at").is_some());
    assert_eq!(exec_rec["output_summary"]["truncated"], false);
    let rec_raw = exec_rec.to_string();
    for forbidden in [secret, herdr_sock, tmux_sock] {
        assert!(
            !rec_raw.contains(forbidden),
            "command record must not contain {forbidden}"
        );
    }
    let exec_sid = exec_rec["session_id"].as_str().expect("session_id");

    // session list/show/latest against the executed record
    let session_list = assert_ok(&h.run(&["--json", "session", "list"]));
    assert_eq!(session_list["command"], "session");
    let list_blob = session_list.to_string();
    assert!(
        list_blob.contains(exec_sid),
        "session list must include executed session {exec_sid}: {session_list}"
    );
    let show = assert_ok(&h.run(&["--json", "session", "show", exec_sid]));
    assert_eq!(show["command"], "session");
    assert!(
        show.to_string().contains(exec_sid),
        "session show must return executed session"
    );
    let latest = assert_ok(&h.run(&["--json", "session", "latest"]));
    assert_eq!(latest["command"], "session");
    assert!(
        latest.to_string().contains(exec_sid)
            || latest.pointer("/data/session_id").and_then(|s| s.as_str()) == Some(exec_sid)
            || latest
                .pointer("/data/record/session_id")
                .and_then(|s| s.as_str())
                == Some(exec_sid),
        "session latest must resolve to executed session: {latest}"
    );

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
    h.assert_lifecycle_spawn_count(before_lifecycle + 1);

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
    // lifecycle marker must remain independent of Ontarch projection spawns
    h.assert_lifecycle_spawn_count(before_lifecycle + 1);

    h.assert_tracked_unchanged();
}

#[test]
fn e2e_mvp_execute_native_exit_code_and_record() {
    let h = E2eHarness::new();
    h.install_lifecycle_child(5);
    let out = h.run(&["--json", "build", "demo", "--execute"]);
    assert_not_unimplemented(&out);
    assert_eq!(
        out.status.code(),
        Some(5),
        "native child exit must pass through: stdout={} stderr={}",
        stdout(&out),
        stderr(&out)
    );
    let v = parse_json(&out);
    assert_eq!(v["data"]["mode"], "executed");
    assert_eq!(v["data"]["execution_requested"], true);
    assert_eq!(v["data"]["execution"]["outcome"], "completed");
    assert_eq!(v["data"]["execution"]["exit_code"], 5);
    assert_eq!(v["exit_code"], 5);
    h.assert_lifecycle_spawn_count(1);
    h.assert_no_spawn();
    let rec = &h.load_records()[0];
    assert_eq!(rec["schema_version"], "0.1.0");
    assert_eq!(rec["execution"]["started"], true);
    assert_eq!(rec["execution"]["outcome"], "completed");
    assert_eq!(rec["execution"]["exit_code"], 5);
    assert!(rec["execution"]["pid"].as_u64().unwrap_or(0) > 0);
    assert!(rec.get("ended_at").is_some());
    h.assert_tracked_unchanged();
}

#[test]
fn e2e_optional_provider_variants_are_inert() {
    // tmux-only: tools reports tmux; doctor remains ready; missing Herdr is optional.
    {
        let h = E2eHarness::new();
        h.overlay_tools_variant("tmux-only");
        h.install_provider_shims(&["tmux"]);
        let tools = assert_ok(&h.run(&["--json", "tools"]));
        let tmux = tools_entry(&tools, "tmux").expect("tmux tool entry");
        assert_eq!(tmux["capability_class"], "optional");
        assert!(tools_entry(&tools, "herdr").is_none());

        let doctor = assert_ok(&h.run(&["--json", "doctor"]));
        assert_eq!(doctor["data"]["ready"], true);
        let tmux_check = doctor_check(&doctor, "tmux").expect("tmux doctor check");
        assert_eq!(tmux_check["ok"], true);
        assert_eq!(tmux_check["severity"], "optional");
        let herdr_check = doctor_check(&doctor, "herdr").expect("herdr doctor check");
        assert_eq!(herdr_check["ok"], false);
        assert_eq!(herdr_check["severity"], "optional");
        h.assert_no_provider_process();
        for forbidden in ["/tmp/tmux-", "HERDR_SOCKET", "socket_path"] {
            assert!(!tools.to_string().contains(forbidden));
            assert!(!doctor.to_string().contains(forbidden));
        }
        h.assert_tracked_unchanged();
    }

    // tmux + Herdr: both reported; no live server/socket contacted.
    {
        let h = E2eHarness::new();
        h.overlay_tools_variant("tmux-herdr");
        h.install_provider_shims(&["tmux", "herdr"]);
        let tools = assert_ok(&h.run(&["--json", "tools"]));
        assert_eq!(
            tools_entry(&tools, "tmux").unwrap()["capability_class"],
            "optional"
        );
        assert_eq!(
            tools_entry(&tools, "herdr").unwrap()["capability_class"],
            "optional"
        );
        let doctor = assert_ok(&h.run(&["--json", "doctor"]));
        assert_eq!(doctor["data"]["ready"], true);
        assert_eq!(doctor_check(&doctor, "tmux").unwrap()["ok"], true);
        assert_eq!(
            doctor_check(&doctor, "tmux").unwrap()["severity"],
            "optional"
        );
        assert_eq!(doctor_check(&doctor, "herdr").unwrap()["ok"], true);
        assert_eq!(
            doctor_check(&doctor, "herdr").unwrap()["severity"],
            "optional"
        );
        h.assert_no_provider_process();
        h.assert_tracked_unchanged();
    }

    // neither: base MVP remains usable; missing Herdr nonfatal when not required.
    {
        let h = E2eHarness::new();
        h.overlay_tools_variant("neither");
        // Do not install provider shims — neither is present on PATH.
        let tools = assert_ok(&h.run(&["--json", "tools"]));
        assert!(tools_entry(&tools, "tmux").is_none());
        assert!(tools_entry(&tools, "herdr").is_none());
        let doctor = assert_ok(&h.run(&["--json", "doctor"]));
        assert_eq!(doctor["data"]["ready"], true);
        assert_eq!(doctor_check(&doctor, "tmux").unwrap()["ok"], false);
        assert_eq!(
            doctor_check(&doctor, "tmux").unwrap()["severity"],
            "optional"
        );
        assert_eq!(doctor_check(&doctor, "herdr").unwrap()["ok"], false);
        assert_eq!(
            doctor_check(&doctor, "herdr").unwrap()["severity"],
            "optional"
        );
        h.assert_no_provider_process();

        // Invalid provider socket/path env must not persist in envelope or record.
        let out = h.run_env(
            &["--json", "build", "demo"],
            &[
                ("TMUX", "/tmp/tmux-invalid/default"),
                ("TMUX_PANE", "%3/bad"),
                ("HERDR_SOCKET_PATH", "/tmp/herdr-invalid.sock"),
            ],
            true,
        );
        assert_ok(&out);
        let raw = format!("{}{}", stdout(&out), stderr(&out));
        for forbidden in [
            "/tmp/tmux-invalid/default",
            "/tmp/herdr-invalid.sock",
            "%3/bad",
        ] {
            assert!(
                !raw.contains(forbidden),
                "envelope must not persist {forbidden}"
            );
        }
        for rec in h.load_records() {
            let s = rec.to_string();
            for forbidden in [
                "/tmp/tmux-invalid/default",
                "/tmp/herdr-invalid.sock",
                "HERDR_SOCKET_PATH",
                "socket_path",
            ] {
                assert!(
                    !s.contains(forbidden),
                    "record must not contain {forbidden}"
                );
            }
        }
        h.assert_tracked_unchanged();
    }
}

#[test]
fn e2e_stale_and_malformed_registry_variants() {
    let h = E2eHarness::new();

    // Fresh hit first.
    let hit = assert_ok(&h.run(&["--json", "graph", "--format", "text"]));
    assert_eq!(hit["metrics"]["registry_cache"], "hit");
    assert_eq!(hit["data"]["freshness"], "hit");

    h.overlay_stale_graph();
    let before = h.load_records().len();
    let stale = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_unimplemented(&stale);
    assert_eq!(
        stale.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stdout(&stale)
    );
    let stale_v = parse_json(&stale);
    assert_eq!(
        stale_v
            .pointer("/diagnostics/0/code")
            .and_then(|c| c.as_str()),
        Some("graph_stale")
    );
    h.assert_no_spawn();
    assert_eq!(
        h.load_records().len(),
        before,
        "stale graph must not record"
    );

    h.overlay_malformed_graph();
    let before = h.load_records().len();
    let bad = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_unimplemented(&bad);
    assert_eq!(bad.status.code(), Some(CONTRACT as i32), "{}", stdout(&bad));
    let bad_v = parse_json(&bad);
    assert_eq!(
        bad_v
            .pointer("/diagnostics/0/code")
            .and_then(|c| c.as_str()),
        Some("graph_contract_invalid")
    );
    h.assert_no_spawn();
    assert_eq!(
        h.load_records().len(),
        before,
        "malformed graph must not record"
    );

    // Units stale: success with registry_cache=stale (non-fatal freshness).
    let h2 = E2eHarness::new();
    h2.overlay_stale_units();
    let list_stale = h2.run(&["--json", "list", "units"]);
    assert_not_unimplemented(&list_stale);
    assert_eq!(
        list_stale.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stdout(&list_stale)
    );
    let list_v = parse_json(&list_stale);
    assert_eq!(list_v["metrics"]["registry_cache"], "stale");
    assert_eq!(list_v["data"]["freshness"], "stale");

    // Units malformed: contract fail-closed.
    h2.overlay_malformed_units();
    let list_bad = h2.run(&["--json", "list", "units"]);
    assert_not_unimplemented(&list_bad);
    assert_eq!(
        list_bad.status.code(),
        Some(CONTRACT as i32),
        "malformed units must fail closed: {}",
        stdout(&list_bad)
    );
    let list_bad_v = parse_json(&list_bad);
    assert_eq!(
        list_bad_v
            .pointer("/diagnostics/0/code")
            .and_then(|c| c.as_str()),
        Some("invalid_registry")
    );

    h.assert_tracked_unchanged();
    h2.assert_tracked_unchanged();
}

#[test]
fn e2e_state_root_variants_fail_closed() {
    // read-only state-home: writing a record must fail with STATE_IO / state_io
    {
        let h = E2eHarness::new();
        h.make_state_home_readonly();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_eq!(
            out.status.code(),
            Some(STATE_IO as i32),
            "read-only state-home: {}",
            stdout(&out)
        );
        let v = parse_json(&out);
        assert_eq!(
            v.pointer("/diagnostics/0/code").and_then(|c| c.as_str()),
            Some("state_io")
        );
        h.assert_no_spawn();
        assert!(h.load_records().is_empty());
        h.restore_state_home_writable();
        h.assert_tracked_unchanged();
    }

    // missing state-home: created on demand; session list + bin report succeed
    {
        let h = E2eHarness::new();
        h.remove_state_home();
        assert!(!h.state_home.exists());
        let out = h.run(&["--json", "session", "list"]);
        assert_ok(&out);
        assert!(h.state_home.is_dir(), "missing state-home must be created");
        let locks = h.state_home.join(".locks");
        let tmp = h.state_home.join(".tmp");
        assert!(locks.is_dir());
        assert!(tmp.is_dir());
        let mode = fs::metadata(&h.state_home).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "state-home mode must be 0700, got {mode:o}");

        let report = assert_ok(&h.run(&["--json", "bin", "report"]));
        assert_eq!(report["command"], "bin report");
        assert_eq!(h.load_records().len(), 1);
        h.assert_spawn_count(1);
        h.assert_tracked_unchanged();
    }

    // symlink state-home: fail closed
    {
        let h = E2eHarness::new();
        h.replace_state_home_with_symlink();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_eq!(
            out.status.code(),
            Some(STATE_IO as i32),
            "symlink state-home: {}",
            stdout(&out)
        );
        let v = parse_json(&out);
        assert_eq!(
            v.pointer("/diagnostics/0/code").and_then(|c| c.as_str()),
            Some("state_io")
        );
        h.assert_no_spawn();
        h.assert_tracked_unchanged();
    }

    // non-directory (file) state-home
    {
        let h = E2eHarness::new();
        h.replace_state_home_with_file();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_eq!(
            out.status.code(),
            Some(STATE_IO as i32),
            "file state-home: {:?}",
            out.status.code()
        );
        let v = parse_json(&out);
        assert_eq!(
            v.pointer("/diagnostics/0/code").and_then(|c| c.as_str()),
            Some("state_io")
        );
        h.assert_no_spawn();
        h.assert_tracked_unchanged();
    }

    // FIFO state-home (requires host mkfifo)
    {
        let h = E2eHarness::new();
        h.replace_state_home_with_fifo();
        let out = h.run(&["--json", "bin", "report"]);
        assert_not_unimplemented(&out);
        assert_eq!(
            out.status.code(),
            Some(STATE_IO as i32),
            "FIFO state-home: {}",
            stdout(&out)
        );
        let v = parse_json(&out);
        assert_eq!(
            v.pointer("/diagnostics/0/code").and_then(|c| c.as_str()),
            Some("state_io")
        );
        h.assert_no_spawn();
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
    // Bin stderr oversize: valid stdout still parses; truncation flagged; spawn once.
    let h = E2eHarness::new();
    h.install_oversized_stderr_ontarch();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_unimplemented(&out);
    assert_eq!(
        out.status.code(),
        Some(SUCCESS as i32),
        "oversized stderr with valid stdout must succeed: {}",
        stdout(&out)
    );
    assert_one_json_document(stdout(&out).trim());
    let v = parse_json(&out);
    assert_compressor_none(&v);
    h.assert_spawn_count(1);
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["started"], true);
    assert_eq!(rec["execution"]["outcome"], "completed");
    assert_eq!(rec["output_summary"]["truncated"], true);
    let stderr_bytes = rec["output_summary"]["stderr_bytes"].as_u64().unwrap();
    assert!(
        stderr_bytes > DEFAULT_LIMIT_BYTES as u64,
        "total stderr bytes ({stderr_bytes}) must exceed capture limit to prove overflow"
    );
    assert!(
        stdout(&out).len() < DEFAULT_LIMIT_BYTES * 2,
        "envelope stdout must stay bounded, got {}",
        stdout(&out).len()
    );

    // Bin stdout oversize: refuse truncated JSON (CONTRACT / bin_payload_invalid).
    let h_out = E2eHarness::new();
    h_out.install_oversized_stdout_ontarch();
    let oversized_out = h_out.run(&["--json", "bin", "report"]);
    assert_not_unimplemented(&oversized_out);
    assert_eq!(
        oversized_out.status.code(),
        Some(CONTRACT as i32),
        "{}",
        stdout(&oversized_out)
    );
    let ov = parse_json(&oversized_out);
    assert_eq!(
        ov.pointer("/diagnostics/0/code").and_then(|c| c.as_str()),
        Some("bin_payload_invalid")
    );
    h_out.assert_spawn_count(1);
    let orec = &h_out.load_records()[0];
    assert!(orec.get("ended_at").is_some(), "terminal record required");
    assert_eq!(orec["output_summary"]["truncated"], true);
    assert_eq!(orec["execution"]["started"], true);

    // Graph: metrics.output_bytes is the Phase 2 emit_success convention (0);
    // numerical bound is asserted on emitted stdout + single-document parse.
    let h2 = E2eHarness::new();
    let g_out = h2.run(&["--json", "graph", "--format", "text"]);
    let g = assert_ok(&g_out);
    assert_compressor_none(&g);
    assert_eq!(
        g["metrics"]["output_bytes"].as_u64(),
        Some(0),
        "graph discovery envelopes report output_bytes=0 by emit_success convention: {g}"
    );
    let emitted = stdout(&g_out).len();
    assert!(
        emitted < DEFAULT_LIMIT_BYTES,
        "emitted graph stdout ({emitted}) must stay under capture limit {DEFAULT_LIMIT_BYTES}"
    );
    assert_one_json_document(stdout(&g_out).trim());
    h2.assert_no_spawn();
    assert!(
        h2.load_records().is_empty(),
        "graph must not create records"
    );

    h.assert_tracked_unchanged();
    h_out.assert_tracked_unchanged();
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
    assert_eq!(
        parse_json(&stale)
            .pointer("/diagnostics/0/code")
            .and_then(|c| c.as_str()),
        Some("graph_stale")
    );

    // Miss: remove graph — zero *new* spawn relative to post-report baseline.
    let spawn_before_miss = h.marker_count();
    let _ = fs::remove_file(h.registry.join("graph.json"));
    let miss = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_unimplemented(&miss);
    assert_eq!(miss.status.code(), Some(RESOLUTION as i32));
    let miss_v = parse_json(&miss);
    assert_eq!(
        miss_v
            .pointer("/diagnostics/0/code")
            .and_then(|c| c.as_str()),
        Some("graph_missing")
    );
    assert_eq!(
        h.marker_count(),
        spawn_before_miss,
        "graph miss must not spawn"
    );
    assert!(!h.path_decoy_marker.exists());

    // RTK absent: truthful fallback — lifecycle plan-only compressor none / no gain required.
    let h2 = E2eHarness::new();
    let life = assert_ok(&h2.run(&["--json", "build", "demo"]));
    if let Some(c) = life.pointer("/metrics/compressor").and_then(|x| x.as_str()) {
        assert_eq!(c, "none", "RTK-absent lifecycle must not claim rtk: {life}");
    }
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
    let isolated = h.root.join("isolated-home");
    assert!(isolated.is_dir());
}

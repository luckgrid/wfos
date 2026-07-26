//! E09.S7 Phase 0 — bin projection acceptance map (§14.2–14.5 / §15).
//!
//! Asserts final S7 contracts. At baseline these fail because `bin` is still
//! `not_implemented` (exit 10) and Ontarch has no `--json` machine stdout.
//! Do not `#[ignore]`.

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use takogami::exit_codes::{
    CONTRACT, EXECUTION_IO, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, STATE_IO, SUCCESS, USAGE,
};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_takogami"))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution")
}

fn stdout(o: &Output) -> &str {
    std::str::from_utf8(&o.stdout).unwrap()
}

fn stderr(o: &Output) -> &str {
    std::str::from_utf8(&o.stderr).unwrap()
}

struct BinHarness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    path_dir: PathBuf,
    marker: PathBuf,
    ontarch_script: PathBuf,
}

impl BinHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        let registry = workspace.join("registry");
        let state_home = temp.path().join("state-home");
        let path_dir = workspace.join("bin");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&path_dir).unwrap();
        copy_dir(&fixture_root(), &workspace);

        let marker = workspace.join("MARKER_RAN");
        let ontarch_script = path_dir.join("ontarch");
        // Default: marker-only Ontarch that writes one valid JSON object to stdout.
        write_json_ontarch(
            &ontarch_script,
            &marker,
            &json!({
                "schema_version": "0.1.0",
                "kind": "bin_inventory",
                "mutation": false,
                "workflows": []
            }),
        );

        Self {
            temp,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
            ontarch_script,
        }
    }

    fn install_ontarch_json(&self, payload: &Value) {
        write_json_ontarch(&self.ontarch_script, &self.marker, payload);
    }

    fn install_ontarch_prose(&self) {
        let script = format!(
            "#!/bin/sh\necho ran >> {}\necho 'human prose inventory'\nexit 0\n",
            self.marker.display()
        );
        fs::write(&self.ontarch_script, script).unwrap();
        fs::set_permissions(&self.ontarch_script, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn install_ontarch_exit(&self, code: i32, stdout_body: &str) {
        let script = format!(
            "#!/bin/sh\necho ran >> {}\nprintf '%s' {stdout}\nexit {code}\n",
            self.marker.display(),
            stdout = shell_single_quote(stdout_body),
            code = code
        );
        fs::write(&self.ontarch_script, script).unwrap();
        fs::set_permissions(&self.ontarch_script, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_env(args, &[])
    }

    fn run_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut cmd = bin();
        cmd.arg("--state-home")
            .arg(&self.state_home)
            .args(args)
            .env("TAKOGAMI_ONTARCH_REGISTRY", &self.registry)
            .env("TAKOGAMI_WORKSPACE_ROOT", &self.workspace)
            .env("TAKOGAMI_STATE_HOME", &self.state_home)
            .env("PATH", &self.path_dir)
            .env_remove("TAKOGAMI_PROFILE")
            .env_remove("XDG_STATE_HOME")
            .env_remove("PANOPLY_AGENT");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn takogami")
    }

    fn marker_count(&self) -> usize {
        if !self.marker.exists() {
            return 0;
        }
        fs::read_to_string(&self.marker)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .count()
    }

    fn assert_marker_untouched(&self) {
        assert_eq!(self.marker_count(), 0, "Ontarch/child must not spawn");
    }

    fn assert_marker_once(&self) {
        assert_eq!(self.marker_count(), 1, "expected exactly one Ontarch spawn");
    }

    fn load_records(&self) -> Vec<Value> {
        if !self.state_home.exists() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.state_home).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            out.push(serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap());
        }
        out
    }
}

fn write_json_ontarch(path: &Path, marker: &Path, payload: &Value) {
    // Capture caller PANOPLY_AGENT into a side file so tests can prove controller override.
    let script = format!(
        "#!/bin/sh\n\
         echo ran >> {marker}\n\
         printf '%s' \"${{PANOPLY_AGENT-}}\" > {marker}.panoply\n\
         printf '%s\\n' {json}\n\
         exit 0\n",
        marker = marker.display(),
        json = shell_single_quote(&payload.to_string()),
    );
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn copy_dir(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&to).unwrap();
            copy_dir(&entry.path(), &to);
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

fn parse_json(out: &Output) -> Value {
    let s = stdout(out);
    serde_json::from_str(s).unwrap_or_else(|e| panic!("JSON parse failed: {e}\nstdout={s}"))
}

fn assert_one_json_document(raw: &str) {
    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    let _ = stream.next().expect("one JSON document").unwrap();
    assert!(
        stream.next().is_none(),
        "exactly one JSON document required"
    );
}

fn assert_not_still_unimplemented(out: &Output) {
    assert_ne!(
        out.status.code(),
        Some(NOT_IMPLEMENTED as i32),
        "S7 bin contracts not implemented yet (exit 10). stderr={}",
        stderr(out)
    );
}

fn record_request_safe(rec: &Value) {
    let flags = rec["request"]["flags"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let joined = flags
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !joined.contains('/'),
        "absolute scope must not appear in record flags: {joined}"
    );
    let raw = serde_json::to_string(rec).unwrap();
    assert!(!raw.contains("SECRET_SENTINEL"));
}

// --- §14.2 request/policy reachability ---

#[test]
fn bin_report_allow_allow_spawns_once_and_records() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_marker_once();
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["schema_version"], "0.1.0");
    assert_eq!(records[0]["record_kind"], "command_execution");
    assert_eq!(records[0]["request"]["command"], "bin report");
    assert!(records[0].get("resolution").is_none());
    record_request_safe(&records[0]);
}

#[test]
fn bin_cleanup_report_only_allow_allow_spawns_once() {
    let h = BinHarness::new();
    h.install_ontarch_json(&json!({
        "schema_version": "0.1.0",
        "kind": "bin_cleanup_plan",
        "mode": "report-only",
        "mutation": false,
        "actions": []
    }));
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_marker_once();
    let rec = &h.load_records()[0];
    assert_eq!(rec["request"]["command"], "bin cleanup");
    record_request_safe(rec);
}

#[test]
fn bin_cleanup_dry_run_is_gate_with_zero_spawns() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "dry-run"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(
        out.status.code(),
        Some(POLICY_GATE as i32),
        "{}",
        stderr(&out)
    );
    h.assert_marker_untouched();
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["policy_decision"]["outcome"], "gate");
    assert!(records[0].get("resolution").is_none());
}

#[test]
fn bin_cleanup_archive_is_deny_deferred_with_zero_spawns() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "archive"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(
        out.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stderr(&out)
    );
    h.assert_marker_untouched();
    let v = parse_json(&out);
    let body = serde_json::to_string(&v).unwrap();
    assert!(
        body.contains("deferred_unavailable") || body.contains("\"deny\""),
        "expected deferred/deny detail: {body}"
    );
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["policy_decision"]["outcome"], "deny");
}

#[test]
fn bin_cleanup_delete_approved_is_deny_deferred_with_zero_spawns() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "delete-approved",
        "--scope",
        "Build/bin/demo",
    ]);
    assert_not_still_unimplemented(&out);
    assert_eq!(
        out.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stderr(&out)
    );
    h.assert_marker_untouched();
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    record_request_safe(&records[0]);
}

#[test]
fn malformed_cleanup_mode_is_usage_or_contract_with_zero_spawns() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "explode"]);
    // Clap rejects unknown enum values before dispatch today; keep that or typed contract.
    assert!(
        matches!(out.status.code(), Some(code) if code == USAGE as i32 || code == CONTRACT as i32 || code == 2),
        "unexpected exit {:?}: {}",
        out.status.code(),
        stderr(&out)
    );
    h.assert_marker_untouched();
}

#[test]
fn invalid_scope_is_usage_or_policy_with_zero_spawns() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "/etc/passwd",
    ]);
    assert_not_still_unimplemented(&out);
    assert!(
        matches!(out.status.code(), Some(code) if code == USAGE as i32 || code == POLICY_DENY as i32 || code == CONTRACT as i32),
        "unexpected exit {:?}: {}",
        out.status.code(),
        stderr(&out)
    );
    h.assert_marker_untouched();
}

#[test]
fn child_executable_drift_does_not_spawn_and_surfaces_projection_contract() {
    let h = BinHarness::new();
    // Remove the sealed Ontarch identity so preflight drift fails closed.
    fs::remove_file(&h.ontarch_script).unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert!(
        matches!(out.status.code(), Some(code) if code == EXECUTION_IO as i32 || code == CONTRACT as i32),
        "unexpected exit {:?}: {}",
        out.status.code(),
        stderr(&out)
    );
    h.assert_marker_untouched();
}

#[test]
fn pending_write_failure_prevents_spawn() {
    let h = BinHarness::new();
    // Make state home a file so pending install cannot create a session record.
    fs::write(&h.state_home, b"not-a-directory").unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(STATE_IO as i32), "{}", stderr(&out));
    // Marker lives beside workspace; state failure must happen before spawn.
    h.assert_marker_untouched();
}

#[test]
fn malformed_policy_registry_fails_closed_with_zero_spawns() {
    let h = BinHarness::new();
    fs::write(h.registry.join("policies.json"), "{broken").unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_marker_untouched();
}

// --- §14.3 child/payload ---

#[test]
fn valid_report_json_completes_with_validated_payload() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_one_json_document(stdout(&out));
    let v = parse_json(&out);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["data"]["execution"]["outcome"], "completed");
}

#[test]
fn valid_report_only_cleanup_json_mutation_false() {
    let h = BinHarness::new();
    h.install_ontarch_json(&json!({
        "schema_version": "0.1.0",
        "kind": "bin_cleanup_plan",
        "mode": "report-only",
        "mutation": false,
        "actions": []
    }));
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["payload"]["mutation"], false);
}

#[test]
fn child_nonzero_exit_preserved_with_terminal_record() {
    let h = BinHarness::new();
    h.install_ontarch_exit(
        7,
        r#"{"schema_version":"0.1.0","kind":"bin_inventory","mutation":false}"#,
    );
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["outcome"], "completed");
    assert_eq!(rec["execution"]["exit_code"], 7);
    assert_eq!(rec["execution"]["started"], true);
}

#[test]
fn child_zero_with_malformed_json_is_controller_error_started_true() {
    let h = BinHarness::new();
    h.install_ontarch_exit(0, "{not-json");
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["started"], true);
    assert_eq!(rec["execution"]["outcome"], "controller_error");
}

#[test]
fn trailing_prose_after_json_is_payload_invalid() {
    let h = BinHarness::new();
    h.install_ontarch_exit(
        0,
        "{\"schema_version\":\"0.1.0\",\"kind\":\"bin_inventory\",\"mutation\":false}\nEXTRA PROSE\n",
    );
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        body.contains("bin_payload_invalid") || body.contains("payload"),
        "expected payload invalid: {body}"
    );
}

#[test]
fn two_json_documents_are_payload_invalid() {
    let h = BinHarness::new();
    h.install_ontarch_exit(0, "{\"a\":1}\n{\"b\":2}\n");
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn invalid_utf8_child_stdout_is_payload_invalid() {
    let h = BinHarness::new();
    let script = format!(
        "#!/bin/sh\necho ran >> {}\nprintf '\\xff\\xfe'\nexit 0\n",
        h.marker.display()
    );
    fs::write(&h.ontarch_script, script).unwrap();
    fs::set_permissions(&h.ontarch_script, fs::Permissions::from_mode(0o755)).unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn stdout_truncation_never_parses_partial_json() {
    let h = BinHarness::new();
    // Emit a huge incomplete JSON object that would exceed capture limits once enforced.
    let huge = format!(
        "{{\"kind\":\"bin_inventory\",\"pad\":\"{}\"",
        "x".repeat(2_000_000)
    );
    h.install_ontarch_exit(0, &huge);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!body.contains("\"status\":\"ok\"") || body.contains("invalid"));
}

#[test]
fn oversized_stderr_stays_bounded() {
    let h = BinHarness::new();
    let script = format!(
        "#!/bin/sh\necho ran >> {}\npython3 -c 'import sys; sys.stderr.write(\"E\"*3000000)'\nprintf '%s\\n' '{{\"schema_version\":\"0.1.0\",\"kind\":\"bin_inventory\",\"mutation\":false}}'\nexit 0\n",
        h.marker.display()
    );
    fs::write(&h.ontarch_script, script).unwrap();
    fs::set_permissions(&h.ontarch_script, fs::Permissions::from_mode(0o755)).unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    // Controller must not hang or dump unbounded stderr into the envelope.
    assert!(stdout(&out).len() < 500_000, "envelope must stay bounded");
}

#[test]
fn cleanup_result_mutation_true_fails_closed() {
    let h = BinHarness::new();
    h.install_ontarch_json(&json!({
        "schema_version": "0.1.0",
        "kind": "bin_cleanup_plan",
        "mode": "report-only",
        "mutation": true,
        "actions": [{"op": "rm", "path": "Build/bin/demo"}]
    }));
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    h.assert_marker_once();
}

#[test]
fn panoply_agent_absent_in_caller_still_fixed_to_one() {
    let h = BinHarness::new();
    let out = h.run_env(&["--json", "bin", "report"], &[]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let seen = fs::read_to_string(format!("{}.panoply", h.marker.display())).unwrap();
    assert_eq!(seen.trim(), "1");
}

#[test]
fn panoply_agent_caller_zero_overridden_to_one() {
    let h = BinHarness::new();
    let out = h.run_env(&["--json", "bin", "report"], &[("PANOPLY_AGENT", "0")]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let seen = fs::read_to_string(format!("{}.panoply", h.marker.display())).unwrap();
    assert_eq!(seen.trim(), "1");
}

#[test]
fn human_mode_machine_output_purity_not_required_but_json_mode_is_pure() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_one_json_document(stdout(&out));
    assert!(!stdout(&out).contains(":: ontarch"));
}

#[test]
fn prose_child_stdout_is_rejected_not_scraped() {
    let h = BinHarness::new();
    h.install_ontarch_prose();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

// --- §14.5 Ontarch direct machine contracts (fail until Phase 1) ---

#[test]
fn ontarch_bin_report_json_emits_one_pure_document() {
    let ontarch =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch/bin/ontarch-bin-report");
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    fs::create_dir_all(&registry).unwrap();
    // Provide a fake WS_ROOT with no bin dirs so the script can still run.
    let ws = temp.path().join("Workstreams");
    fs::create_dir_all(ws.join("Build/bin")).unwrap();
    let out = Command::new(&ontarch)
        .arg("--json")
        .env("ONTARCH_REGISTRY", &registry)
        .env("WS_ROOT", &ws)
        .output()
        .expect("spawn ontarch-bin-report");
    assert!(
        out.status.success(),
        "ontarch bin-report --json must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let raw = std::str::from_utf8(&out.stdout).unwrap();
    assert_one_json_document(raw);
    assert!(!raw.contains(":: ontarch"));
}

#[test]
fn ontarch_bin_cleanup_report_only_json_emits_one_pure_document() {
    let ontarch =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch/bin/ontarch-bin-cleanup");
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    fs::create_dir_all(&registry).unwrap();
    // Minimal inventory so cleanup does not need to refresh.
    fs::write(
        registry.join("bin-inventory.json"),
        r#"{"generated_at":"2026-07-25T00:00:00Z","summary":{"total":0,"with_manifest":0},"workflows":[]}"#,
    )
    .unwrap();
    let ws = temp.path().join("Workstreams");
    fs::create_dir_all(&ws).unwrap();
    let out = Command::new(&ontarch)
        .args(["--mode", "report-only", "--json"])
        .env("ONTARCH_REGISTRY", &registry)
        .env("WS_ROOT", &ws)
        .output()
        .expect("spawn ontarch-bin-cleanup");
    assert!(
        out.status.success(),
        "ontarch bin-cleanup --json must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let raw = std::str::from_utf8(&out.stdout).unwrap();
    assert_one_json_document(raw);
    let v: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(v["mutation"], false);
}

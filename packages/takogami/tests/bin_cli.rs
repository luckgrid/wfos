//! E09.S7 Phase 0 — bin projection acceptance map (review-corrected).
//!
//! Canonical Ontarch package layout (not PATH authority). Plan-aligned payloads.
//! Failures today: `bin` still `not_implemented` (exit 10). Do not `#[ignore]`.

#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use support::{
    copy_dir, sample_cleanup_mutation_true, sample_cleanup_plan, sample_inventory,
    shell_single_quote, write_canonical_fake_ontarch, write_executable, write_marker_exe,
};
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

/// Closed projection source set required for seal (logical packages/ontarch/… labels).
fn ensure_projection_source_manifest(ontarch_pkg: &std::path::Path) {
    let real = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch");
    let copy_or_stub = |rel: &str, stub: &[u8]| {
        let dest = ontarch_pkg.join(rel);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let src = real.join(rel);
        if src.is_file() {
            fs::copy(&src, &dest).unwrap();
        } else {
            fs::write(&dest, stub).unwrap();
        }
    };
    // Bins already include ontarch; ensure siblings exist as regular files.
    if !ontarch_pkg.join("bin/ontarch-bin-report").is_file() {
        write_executable(
            &ontarch_pkg.join("bin/ontarch-bin-report"),
            "#!/bin/sh\nexit 0\n",
        );
    }
    if !ontarch_pkg.join("bin/ontarch-bin-cleanup").is_file() {
        write_executable(
            &ontarch_pkg.join("bin/ontarch-bin-cleanup"),
            "#!/bin/sh\nexit 0\n",
        );
    }
    copy_or_stub("lib/common.sh", b"# test\n");
    copy_or_stub("lib/registry.sh", b"# test\n");
    copy_or_stub("policies/takogami.agent.policy.toml", b"# test\n");
    copy_or_stub("policies/agent-bin.policy.toml", b"# test\n");
    copy_or_stub("schemas/bin-inventory.schema.json", b"{}\n");
    copy_or_stub("schemas/bin-cleanup-plan.schema.json", b"{}\n");
}

struct BinHarness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    /// WfOS-like workspace root containing `packages/ontarch/`.
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    /// Approved tools PATH (decoy ontarch may live here; must be ignored).
    path_dir: PathBuf,
    marker: PathBuf,
    path_decoy_marker: PathBuf,
    panoply_side: PathBuf,
    canonical_ontarch: PathBuf,
}

impl BinHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        let ontarch_pkg = workspace.join("packages/ontarch");
        let registry = ontarch_pkg.join("registry");
        let state_home = temp.path().join("state-home");
        let path_dir = temp.path().join("path-tools");
        fs::create_dir_all(ontarch_pkg.join("bin")).unwrap();
        fs::create_dir_all(&registry).unwrap();
        fs::create_dir_all(&path_dir).unwrap();

        // Resolution policies/profiles/units under the Ontarch registry path.
        copy_dir(&fixture_root().join("registry"), &registry);
        if fixture_root().join("demo").is_dir() {
            copy_dir(&fixture_root().join("demo"), &workspace.join("demo"));
        }
        // Descriptor sources expected by some resolution helpers.
        let sources = fixture_root().join("registry/sources");
        if sources.is_dir() {
            copy_dir(&sources, &registry.join("sources"));
        }

        let marker = workspace.join("MARKER_CANONICAL");
        let path_decoy_marker = workspace.join("MARKER_PATH_DECOY");
        let panoply_side = workspace.join("PANOPLY_SEEN");
        let canonical_ontarch = ontarch_pkg.join("bin/ontarch");
        let inv = sample_inventory(workspace.to_str().unwrap());
        let clean = sample_cleanup_plan("report-only");
        write_canonical_fake_ontarch(&canonical_ontarch, &marker, &panoply_side, &inv, &clean);
        ensure_projection_source_manifest(&ontarch_pkg);
        // PATH decoy — must never run when canonical package Ontarch is authoritative.
        write_marker_exe(&path_dir.join("ontarch"), &path_decoy_marker);

        Self {
            temp,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
            path_decoy_marker,
            panoply_side,
            canonical_ontarch,
        }
    }

    fn install_ontarch_json(&self, inventory: &Value, cleanup: &Value) {
        write_canonical_fake_ontarch(
            &self.canonical_ontarch,
            &self.marker,
            &self.panoply_side,
            inventory,
            cleanup,
        );
    }

    fn install_ontarch_stdout(&self, body: &str, exit: i32) {
        let out_file = self.workspace.join("child_stdout.bin");
        fs::write(&out_file, body.as_bytes()).unwrap();
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\necho ran >> {m}\nprintf '%s' \"${{PANOPLY_AGENT-}}\" > {p}\ncat {out}\nexit {e}\n",
                m = shell_single_quote(&self.marker.to_string_lossy()),
                p = shell_single_quote(&self.panoply_side.to_string_lossy()),
                out = shell_single_quote(&out_file.to_string_lossy()),
                e = exit
            ),
        );
    }

    fn install_ontarch_bytes(&self, bytes: &[u8], exit: i32) {
        let out_file = self.workspace.join("child_stdout.bin");
        fs::write(&out_file, bytes).unwrap();
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\necho ran >> {m}\nprintf '%s' \"${{PANOPLY_AGENT-}}\" > {p}\ncat {out}\nexit {e}\n",
                m = shell_single_quote(&self.marker.to_string_lossy()),
                p = shell_single_quote(&self.panoply_side.to_string_lossy()),
                out = shell_single_quote(&out_file.to_string_lossy()),
                e = exit
            ),
        );
    }

    fn install_ontarch_prose(&self) {
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\necho ran >> {m}\necho 'human prose inventory'\nexit 0\n",
                m = shell_single_quote(&self.marker.to_string_lossy())
            ),
        );
    }

    /// Child dies by signal (default disposition) so the executor records interrupted + 128+sig.
    fn install_ontarch_self_signal(&self, signal_name: &str) {
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\necho ran >> {m}\nkill -s {sig} $$\n",
                m = shell_single_quote(&self.marker.to_string_lossy()),
                sig = signal_name
            ),
        );
    }

    fn install_ontarch_oversized_stderr(&self) {
        let good = sample_inventory(self.workspace.to_str().unwrap()).to_string();
        let out_file = self.workspace.join("child_stdout.bin");
        fs::write(&out_file, good.as_bytes()).unwrap();
        // Shell builtin loop — no python3 dependency on isolated PATH.
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\n\
                 echo ran >> {m}\n\
                 i=0\n\
                 while [ \"$i\" -lt 300000 ]; do\n\
                   printf 'E' >&2\n\
                   i=$((i+1))\n\
                 done\n\
                 cat {out}\n\
                 exit 0\n",
                m = shell_single_quote(&self.marker.to_string_lossy()),
                out = shell_single_quote(&out_file.to_string_lossy()),
            ),
        );
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
        assert_eq!(self.marker_count(), 0, "canonical Ontarch must not spawn");
        assert!(
            !self.path_decoy_marker.exists(),
            "PATH decoy ontarch must never run"
        );
    }

    fn assert_marker_once(&self) {
        assert_eq!(
            self.marker_count(),
            1,
            "expected exactly one canonical spawn"
        );
        assert!(
            !self.path_decoy_marker.exists(),
            "PATH decoy ontarch must never run"
        );
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

fn assert_allow_record(rec: &Value) {
    assert_eq!(rec["schema_version"], "0.1.0");
    assert_eq!(rec["record_kind"], "command_execution");
    assert!(rec.get("resolution").is_none());
    assert_eq!(rec["policy_decision"]["outcome"], "allow");
    let digest = rec["plan_digest"].as_str().unwrap_or("");
    assert!(
        digest.starts_with("sha256:"),
        "plan_digest must be sha256:… got {digest}"
    );
    assert_eq!(rec["execution"]["started"], true);
    assert!(rec["execution"]["pid"].as_u64().unwrap_or(0) > 0);
    assert!(rec.get("ended_at").is_some());
    let raw = serde_json::to_string(rec).unwrap();
    assert!(!raw.contains("SECRET_SENTINEL"));
    assert!(!raw.contains("/packages/ontarch/bin/"));
}

fn assert_gate_or_deny_record(rec: &Value, outcome: &str) {
    assert_eq!(rec["policy_decision"]["outcome"], outcome);
    assert_eq!(rec["execution"]["started"], false);
    assert!(rec["execution"]["pid"].is_null());
    assert!(rec.get("resolution").is_none());
    assert!(
        rec["execution"]
            .get("exit_code")
            .map(|v| v.is_null())
            .unwrap_or(true)
            || rec["execution"]["exit_code"].is_null()
    );
}

fn assert_no_absolute_leak(out: &Output, records: &[Value], needle: &str) {
    let body = format!("{}{}", stdout(out), stderr(out));
    assert!(
        !body.contains(needle),
        "absolute operand must not leak to output"
    );
    for rec in records {
        let raw = serde_json::to_string(rec).unwrap();
        assert!(!raw.contains(needle), "absolute operand must not persist");
    }
}

// --- §14.2 request/policy ---

#[test]
fn bin_report_allow_allow_spawns_once_and_records() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_marker_once();
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["request"]["command"], "bin report");
    assert_allow_record(&records[0]);
}

#[test]
fn bin_cleanup_report_only_allow_allow_spawns_once() {
    let h = BinHarness::new();
    h.install_ontarch_json(
        &sample_inventory(h.workspace.to_str().unwrap()),
        &sample_cleanup_plan("report-only"),
    );
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_marker_once();
    assert_allow_record(&h.load_records()[0]);
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
    assert_gate_or_deny_record(&records[0], "gate");
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
        body.contains("deferred_unavailable"),
        "archive must include deferred_unavailable detail: {body}"
    );
    assert_eq!(v["data"]["policy_decision"]["outcome"], "deny");
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    assert_gate_or_deny_record(&records[0], "deny");
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
    let body = serde_json::to_string(&parse_json(&out)).unwrap();
    assert!(body.contains("deferred_unavailable"));
    assert_gate_or_deny_record(&h.load_records()[0], "deny");
}

#[test]
fn malformed_cleanup_mode_is_usage_or_contract_with_zero_spawns() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "explode"]);
    assert!(
        matches!(out.status.code(), Some(code) if code == USAGE as i32 || code == CONTRACT as i32 || code == 2),
        "unexpected exit {:?}: {}",
        out.status.code(),
        stderr(&out)
    );
    h.assert_marker_untouched();
}

#[test]
fn invalid_scope_syntax_is_usage_or_contract_with_no_policy_record() {
    let h = BinHarness::new();
    let abs = "/etc/passwd";
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        abs,
    ]);
    assert_not_still_unimplemented(&out);
    assert!(
        matches!(out.status.code(), Some(code) if code == USAGE as i32 || code == CONTRACT as i32),
        "invalid absolute scope must not reach policy Deny; got {:?}: {}",
        out.status.code(),
        stderr(&out)
    );
    h.assert_marker_untouched();
    let records = h.load_records();
    assert!(
        records.is_empty(),
        "pre-policy invalid scope must not create a policy decision record"
    );
    assert_no_absolute_leak(&out, &records, abs);
}

#[test]
fn valid_but_policy_blocked_scope_is_deny_with_safe_record() {
    let h = BinHarness::new();
    // Grammar-valid scope blocked by an injected path deny rule.
    let policies = h.registry.join("policies.json");
    let mut doc: Value = serde_json::from_str(&fs::read_to_string(&policies).unwrap()).unwrap();
    for pol in doc["policies"].as_array_mut().unwrap() {
        if pol["id"] == "agent-bin" {
            pol["block"]["paths"] = serde_json::json!(["bin/", "lib/", "Brand/bin/**"]);
        }
    }
    fs::write(&policies, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "Brand/bin/blocked",
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
    assert_gate_or_deny_record(&records[0], "deny");
}

#[test]
fn lib_or_src_scope_is_rejected_before_policy() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "lib/secret",
    ]);
    assert_not_still_unimplemented(&out);
    assert!(
        matches!(out.status.code(), Some(code) if code == USAGE as i32 || code == CONTRACT as i32),
        "lib/src scope must fail pre-policy; got {:?}: {}",
        out.status.code(),
        stderr(&out)
    );
    assert!(h.load_records().is_empty());
    h.assert_marker_untouched();
}

#[test]
fn missing_canonical_ontarch_fails_before_spawn_even_when_path_replacement_exists() {
    let h = BinHarness::new();
    fs::remove_file(&h.canonical_ontarch).unwrap();
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
fn projection_executable_identity_drift_after_seal_fails_preflight() {
    let h = BinHarness::new();
    write_executable(
        &h.path_dir.join("ontarch"),
        "#!/bin/sh\necho path-rescue\nexit 0\n",
    );
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_EXE_DRIFT", "1")],
    );
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    assert!(
        !h.path_decoy_marker.exists(),
        "must not rescue via PATH after identity loss"
    );
    h.assert_marker_untouched();
}

#[test]
fn projection_cwd_identity_drift_after_seal_fails_preflight() {
    let h = BinHarness::new();
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_CWD_DRIFT", "1")],
    );
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    h.assert_marker_untouched();
}

#[test]
fn pending_write_failure_prevents_spawn() {
    let h = BinHarness::new();
    fs::write(&h.state_home, b"not-a-directory").unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(STATE_IO as i32), "{}", stderr(&out));
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

// --- payload ---

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
fn valid_report_only_cleanup_json_mutation_executed_false() {
    let h = BinHarness::new();
    h.install_ontarch_json(
        &sample_inventory(h.workspace.to_str().unwrap()),
        &sample_cleanup_plan("report-only"),
    );
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["payload"]["mutation_executed"], false);
}

#[test]
fn child_nonzero_exit_preserved_with_terminal_record() {
    let h = BinHarness::new();
    let payload = sample_inventory(h.workspace.to_str().unwrap()).to_string();
    h.install_ontarch_stdout(&payload, 7);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["outcome"], "completed");
    assert_eq!(rec["execution"]["exit_code"], 7);
    assert_eq!(rec["execution"]["started"], true);
}

#[test]
fn child_signal_exit_preserved_with_terminal_record() {
    let h = BinHarness::new();
    h.install_ontarch_self_signal("TERM");
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(143), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["outcome"], "interrupted");
    assert_eq!(rec["execution"]["signal"], "SIGTERM");
    assert_eq!(rec["execution"]["exit_code"], 143);
    assert_eq!(rec["execution"]["started"], true);
    h.assert_marker_once();
}

#[test]
fn broken_pipe_on_bin_report_json_is_success_and_finalizes() {
    let h = BinHarness::new();
    let mut child = bin()
        .arg("--state-home")
        .arg(&h.state_home)
        .args(["--json", "bin", "report"])
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &h.workspace)
        .env("TAKOGAMI_STATE_HOME", &h.state_home)
        .env("PATH", &h.path_dir)
        .env_remove("TAKOGAMI_PROFILE")
        .env_remove("XDG_STATE_HOME")
        .env_remove("PANOPLY_AGENT")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdout.take());
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(SUCCESS as i32),
        "broken pipe must not override Allow success"
    );
    assert_eq!(h.load_records().len(), 1);
    assert_eq!(h.load_records()[0]["execution"]["outcome"], "completed");
    h.assert_marker_once();
}

#[test]
fn broken_pipe_on_bin_gate_preserves_policy_exit() {
    let h = BinHarness::new();
    let mut child = bin()
        .arg("--state-home")
        .arg(&h.state_home)
        .args(["--json", "bin", "cleanup", "--mode", "dry-run"])
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &h.workspace)
        .env("TAKOGAMI_STATE_HOME", &h.state_home)
        .env("PATH", &h.path_dir)
        .env_remove("TAKOGAMI_PROFILE")
        .env_remove("XDG_STATE_HOME")
        .env_remove("PANOPLY_AGENT")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdout.take());
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(POLICY_GATE as i32),
        "broken pipe must preserve Gate exit"
    );
    h.assert_marker_untouched();
    assert_eq!(h.load_records().len(), 1);
}

#[test]
fn child_zero_with_malformed_json_is_controller_error_started_true() {
    let h = BinHarness::new();
    h.install_ontarch_stdout("{not-json", 0);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["started"], true);
    assert!(rec["execution"]["pid"].as_u64().unwrap_or(0) > 0);
    assert_eq!(rec["execution"]["outcome"], "controller_error");
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(body.contains("bin_payload_invalid") || body.contains("payload"));
}

#[test]
fn trailing_prose_after_json_is_payload_invalid() {
    let h = BinHarness::new();
    let payload = format!(
        "{}\nEXTRA PROSE\n",
        sample_inventory(h.workspace.to_str().unwrap())
    );
    h.install_ontarch_stdout(&payload, 0);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn two_json_documents_are_payload_invalid() {
    let h = BinHarness::new();
    h.install_ontarch_stdout("{\"a\":1}\n{\"b\":2}\n", 0);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn invalid_utf8_child_stdout_is_payload_invalid() {
    let h = BinHarness::new();
    h.install_ontarch_bytes(&[0xff, 0xfe], 0);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn stdout_truncation_never_parses_partial_json() {
    let h = BinHarness::new();
    let huge = format!(
        "{{\"generated_at\":\"2026-07-25T00:00:00Z\",\"root\":\"x\",\"summary\":{{\"total\":0,\"with_manifest\":0}},\"workflows\":[],\"pad\":\"{}\"",
        "x".repeat(2_000_000)
    );
    h.install_ontarch_stdout(&huge, 0);
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn oversized_stderr_stays_bounded() {
    let h = BinHarness::new();
    h.install_ontarch_oversized_stderr();
    let out = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&out);
    assert!(stdout(&out).len() < 500_000, "envelope must stay bounded");
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["started"], true);
}

#[test]
fn cleanup_result_mutation_executed_true_fails_closed() {
    let h = BinHarness::new();
    h.install_ontarch_json(
        &sample_inventory(h.workspace.to_str().unwrap()),
        &sample_cleanup_mutation_true(),
    );
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
    let seen = fs::read_to_string(&h.panoply_side).unwrap();
    assert_eq!(seen.trim(), "1");
}

#[test]
fn panoply_agent_caller_zero_overridden_to_one() {
    let h = BinHarness::new();
    let out = h.run_env(&["--json", "bin", "report"], &[("PANOPLY_AGENT", "0")]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let seen = fs::read_to_string(&h.panoply_side).unwrap();
    assert_eq!(seen.trim(), "1");
}

#[test]
fn json_mode_machine_output_is_pure() {
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

#[test]
fn scope_absent_allows_workspace_wide_report_only() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "cleanup", "--mode", "report-only"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    assert!(
        !rec["request"]["flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "scope_provided")
    );
}

#[test]
fn workflow_scope_is_accepted_and_forwarded_literally() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "Build/bin/wfos",
    ]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    assert!(
        rec["request"]["flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "scope_provided")
    );
    assert_no_absolute_leak(&out, &h.load_records(), h.workspace.to_str().unwrap());
}

#[test]
fn namespace_root_scope_plan_bin_is_rejected() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "Plan/bin",
    ]);
    assert_not_still_unimplemented(&out);
    assert!(matches!(
        out.status.code(),
        Some(code) if code == USAGE as i32 || code == CONTRACT as i32
    ));
    assert!(h.load_records().is_empty());
    h.assert_marker_untouched();
}

#[test]
fn namespace_root_scope_build_bin_is_rejected() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "Build/bin",
    ]);
    assert_not_still_unimplemented(&out);
    assert!(matches!(
        out.status.code(),
        Some(code) if code == USAGE as i32 || code == CONTRACT as i32
    ));
    assert!(h.load_records().is_empty());
}

#[test]
fn scope_with_traversal_is_rejected_before_policy() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "Build/bin/../lib",
    ]);
    assert!(h.load_records().is_empty());
    h.assert_marker_untouched();
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
}

#[test]
fn validated_scope_record_stores_only_scope_provided() {
    let h = BinHarness::new();
    let out = h.run(&[
        "--json",
        "bin",
        "cleanup",
        "--mode",
        "report-only",
        "--scope",
        "Build/bin/demo",
    ]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    let blob = serde_json::to_string(rec).unwrap();
    assert!(blob.contains("scope_provided"));
    assert!(!blob.contains("Build/bin/demo"));
}

// --- E09.S7 Phase 3 closure corrections (C01–C05) ---

fn write_helper_decoy(path_dir: &std::path::Path, name: &str, marker: &std::path::Path) {
    write_marker_exe(&path_dir.join(name), marker);
}

#[test]
fn caller_path_decoy_jq_does_not_run() {
    let h = BinHarness::new();
    let decoy = h.workspace.join("MARKER_JQ");
    write_helper_decoy(&h.path_dir, "jq", &decoy);
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_marker_once();
    assert!(!decoy.exists(), "caller PATH decoy jq must never run");
}

#[test]
fn caller_path_decoy_sed_does_not_run() {
    let h = BinHarness::new();
    let decoy = h.workspace.join("MARKER_SED");
    write_helper_decoy(&h.path_dir, "sed", &decoy);
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(!decoy.exists(), "caller PATH decoy sed must never run");
}

#[test]
fn caller_path_decoy_find_or_fd_does_not_run() {
    let h = BinHarness::new();
    let decoy_find = h.workspace.join("MARKER_FIND");
    let decoy_fd = h.workspace.join("MARKER_FD");
    write_helper_decoy(&h.path_dir, "find", &decoy_find);
    write_helper_decoy(&h.path_dir, "fd", &decoy_fd);
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(!decoy_find.exists());
    assert!(!decoy_fd.exists());
}

#[test]
fn projection_source_digest_drift_after_authorization_fails_preflight() {
    let h = BinHarness::new();
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_SOURCE_DRIFT", "1")],
    );
    assert_ne!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_marker_untouched();
    let blob = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        !blob.contains(h.workspace.to_string_lossy().as_ref()),
        "diagnostics must omit absolute workspace root: {blob}"
    );
}

#[test]
fn projection_source_removed_after_authorization_fails_preflight() {
    // Seal-time sources exist; remove a required file via drift of common.sh content then
    // separately prove missing-at-seal via a dedicated harness mutation.
    let h = BinHarness::new();
    let common = h.workspace.join("packages/ontarch/lib/common.sh");
    fs::remove_file(&common).unwrap();
    let out = h.run(&["--json", "bin", "report"]);
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    h.assert_marker_untouched();
}

#[test]
fn projection_source_replaced_same_length_fails_preflight() {
    let h = BinHarness::new();
    let common = h.workspace.join("packages/ontarch/lib/common.sh");
    // Same length as inject seam body (`# drifted\n` = 10 bytes), different digest.
    fs::write(&common, b"#DRIFTED?\n").unwrap();
    assert_eq!(fs::read(&common).unwrap().len(), b"# drifted\n".len());
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_SOURCE_DRIFT", "1")],
    );
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    h.assert_marker_untouched();
}

#[test]
fn source_drift_never_runs_path_decoy_or_canonical_child() {
    let h = BinHarness::new();
    let decoy = h.workspace.join("MARKER_JQ");
    write_helper_decoy(&h.path_dir, "jq", &decoy);
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_SOURCE_DRIFT", "1")],
    );
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    h.assert_marker_untouched();
    assert!(!decoy.exists());
}

#[test]
fn source_drift_diagnostic_omits_absolute_roots() {
    let h = BinHarness::new();
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_SOURCE_DRIFT", "1")],
    );
    let blob = format!("{}{}", stdout(&out), stderr(&out));
    assert!(!blob.contains(h.workspace.to_string_lossy().as_ref()));
    assert!(!blob.contains(h.registry.to_string_lossy().as_ref()));
}

#[test]
fn human_bin_report_is_not_json() {
    let h = BinHarness::new();
    let out = h.run(&["bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let s = stdout(&out);
    assert!(
        !s.trim_start().starts_with('{'),
        "human output must not be JSON: {s}"
    );
    assert!(s.contains("Bin inventory"), "{s}");
    assert!(!s.contains(h.workspace.to_string_lossy().as_ref()));
}

#[test]
fn human_cleanup_report_only_is_not_json() {
    let h = BinHarness::new();
    let out = h.run(&["bin", "cleanup", "--mode", "report-only"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let s = stdout(&out);
    assert!(!s.trim_start().starts_with('{'));
    assert!(
        s.contains("report-only") || s.contains("Bin cleanup"),
        "{s}"
    );
    assert!(
        s.contains("Mutation executed: false") || s.contains("mutation"),
        "{s}"
    );
}

#[test]
fn human_cleanup_gate_is_not_json() {
    let h = BinHarness::new();
    let out = h.run(&["bin", "cleanup", "--mode", "dry-run"]);
    assert_eq!(
        out.status.code(),
        Some(POLICY_GATE as i32),
        "{}",
        stderr(&out)
    );
    let s = stdout(&out);
    assert!(!s.trim_start().starts_with('{'), "{s}");
    h.assert_marker_untouched();
}

#[test]
fn human_cleanup_deny_is_not_json() {
    let h = BinHarness::new();
    let out = h.run(&["bin", "cleanup", "--mode", "archive"]);
    assert_eq!(
        out.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stderr(&out)
    );
    let s = stdout(&out);
    assert!(!s.trim_start().starts_with('{'), "{s}");
    h.assert_marker_untouched();
}

#[test]
fn json_bin_report_is_exactly_one_envelope() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32));
    let v: Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert!(v.get("schema_version").is_some());
    assert!(stdout(&out).trim().lines().count() >= 1);
}

#[test]
fn json_cleanup_outcomes_are_exactly_one_envelope() {
    let h = BinHarness::new();
    for (args, code) in [
        (
            &["--json", "bin", "cleanup", "--mode", "report-only"][..],
            SUCCESS,
        ),
        (
            &["--json", "bin", "cleanup", "--mode", "dry-run"][..],
            POLICY_GATE,
        ),
        (
            &["--json", "bin", "cleanup", "--mode", "archive"][..],
            POLICY_DENY,
        ),
    ] {
        let out = h.run(args);
        assert_eq!(out.status.code(), Some(code as i32), "{args:?}");
        let _: Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    }
}

#[test]
fn human_output_is_bounded() {
    let h = BinHarness::new();
    let out = h.run(&["bin", "report"]);
    let s = stdout(&out);
    assert!(s.lines().count() < 40, "human output too large: {s}");
    assert!(!s.contains("\"workflows\""));
}

#[test]
fn human_output_omits_absolute_roots() {
    let h = BinHarness::new();
    let out = h.run(&["bin", "report"]);
    let s = stdout(&out);
    assert!(!s.contains(h.workspace.to_string_lossy().as_ref()));
    assert!(!s.contains(h.registry.to_string_lossy().as_ref()));
}

#[test]
fn projection_terminal_retains_pending_started_at() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    let started = rec["started_at"].as_str().unwrap();
    let ended = rec["ended_at"].as_str().unwrap();
    assert!(started.ends_with('Z') && started.len() == 20);
    assert!(ended.ends_with('Z') && ended.len() == 20);
    // Terminal derives from pending: started_at must remain a seal-time identity field
    // (not absent / not rewritten to null). Same-second started/ended is allowed.
    assert_eq!(rec["schema_version"], "0.1.0");
    assert!(rec.get("resolution").is_none() || rec["resolution"].is_null());
}

#[test]
fn projection_terminal_retains_request_policy_and_fingerprints() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32));
    let rec = &h.load_records()[0];
    assert!(rec.get("request").is_some());
    assert!(rec.get("policy_decision").is_some());
    assert!(
        rec["source_fingerprints"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    );
    let fps = rec["source_fingerprints"].as_array().unwrap();
    for fp in fps {
        let path = fp["path"].as_str().unwrap();
        assert!(path.starts_with("packages/ontarch/"));
        assert!(!path.starts_with('/'));
    }
}

#[test]
fn projection_preflight_failure_persists_safe_error() {
    let h = BinHarness::new();
    let out = h.run_env(
        &["--json", "bin", "report"],
        &[("TAKOGAMI_TEST_INJECT_SOURCE_DRIFT", "1")],
    );
    assert_ne!(out.status.code(), Some(SUCCESS as i32));
    // Pending may remain if written before preflight inside executor.
    let records = h.load_records();
    if let Some(rec) = records.first() {
        if let Some(err) = rec.get("error") {
            let blob = err.to_string();
            assert!(!blob.contains(h.workspace.to_string_lossy().as_ref()));
        }
    }
}

#[test]
fn projection_payload_error_persists_child_exit_zero_and_contract_error() {
    let h = BinHarness::new();
    h.install_ontarch_prose();
    ensure_projection_source_manifest(&h.workspace.join("packages/ontarch"));
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["exit_code"], 0);
    assert!(rec.get("error").is_some());
}

#[test]
fn projection_output_summary_merges_stdout_stderr_encoding() {
    let h = BinHarness::new();
    let out = h.run(&["--json", "bin", "report"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32));
    let rec = &h.load_records()[0];
    let enc = rec["output_summary"]["encoding"].as_str().unwrap();
    assert!(matches!(enc, "utf-8" | "lossy-utf-8" | "binary"));
}

//! E09.S7 Phase 0 — hermetic MVP acceptance skeleton (§13 / §14.4 / §15).
//!
//! Shared fixture builder + focused path proofs. Full content fills in Phases 2–4.
//! Failures today are expected: graph/bin remain `not_implemented`.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use takogami::exit_codes::{NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, SUCCESS};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_takogami"))
}

fn e2e_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e2e")
}

fn resolution_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution")
}

fn stdout(o: &Output) -> &str {
    std::str::from_utf8(&o.stdout).unwrap()
}

fn stderr(o: &Output) -> &str {
    std::str::from_utf8(&o.stderr).unwrap()
}

struct E2eHarness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    root: PathBuf,
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    path_dir: PathBuf,
}

impl E2eHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("e2e");
        fs::create_dir_all(&root).unwrap();
        copy_dir(&e2e_root(), &root);
        // Overlay a working resolution registry so discovery/lifecycle paths can run.
        let registry = root.join("ontarch/registry");
        copy_dir(&resolution_root().join("registry"), &registry);
        let workspace = root.join("workspace");
        // Provide demo unit sources expected by resolution descriptors.
        copy_dir(&resolution_root().join("demo"), &workspace.join("demo"));
        let state_home = root.join("state");
        fs::create_dir_all(&state_home).unwrap();
        let path_dir = root.join("tools");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["cargo", "rustc", "moon", "ontarch", "demo-bin", "rg", "git"] {
            write_ok_exe(&path_dir.join(name));
        }
        Self {
            temp,
            root,
            workspace,
            registry,
            state_home,
            path_dir,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        bin()
            .arg("--state-home")
            .arg(&self.state_home)
            .args(args)
            .env("TAKOGAMI_ONTARCH_REGISTRY", &self.registry)
            .env("TAKOGAMI_WORKSPACE_ROOT", &self.workspace)
            .env("TAKOGAMI_STATE_HOME", &self.state_home)
            .env("PATH", &self.path_dir)
            .env_remove("TAKOGAMI_PROFILE")
            .env_remove("XDG_STATE_HOME")
            .output()
            .expect("spawn takogami")
    }

    fn load_records(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if !self.state_home.exists() {
            return out;
        }
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

fn write_ok_exe(path: &Path) {
    fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

fn assert_not_still_unimplemented(out: &Output) {
    assert_ne!(
        out.status.code(),
        Some(NOT_IMPLEMENTED as i32),
        "S7 MVP path not implemented yet (exit 10). stderr={}",
        stderr(out)
    );
}

#[test]
fn e2e_fixture_skeleton_is_tracked_and_copyable() {
    let root = e2e_root();
    for rel in [
        "README.md",
        "workspace/Build/bin/demo/manifest.json",
        "workspace/Plan/bin/stale-demo/manifest.json",
        "ontarch/registry/graph.json",
        "ontarch/registry/graph.dot",
        "expected/graph-text.txt",
        "expected/bin-report-envelope.json",
    ] {
        assert!(root.join(rel).is_file(), "missing tracked fixture {rel}");
    }
    let h = E2eHarness::new();
    assert!(h.root.join("ontarch/registry/graph.json").is_file());
    // Repository copy must remain unchanged after harness construction.
    let tracked = fs::read_to_string(e2e_root().join("ontarch/registry/graph.json")).unwrap();
    assert!(tracked.contains("\"demo\""));
}

#[test]
fn e2e_discovery_and_lifecycle_still_work_on_overlay() {
    let h = E2eHarness::new();
    let out = h.run(&["--json", "doctor"]);
    // Doctor may succeed or fail on missing required tools depending on PATH isolation;
    // it must never be not_implemented.
    assert_ne!(out.status.code(), Some(NOT_IMPLEMENTED as i32));
    let out = h.run(&["--json", "build", "demo"]);
    assert_ne!(
        out.status.code(),
        Some(NOT_IMPLEMENTED as i32),
        "{}",
        stderr(&out)
    );
}

#[test]
fn e2e_mvp_path_includes_graph_projection() {
    let h = E2eHarness::new();
    let out = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(h.load_records().is_empty(), "graph creates no records");
}

#[test]
fn e2e_mvp_path_includes_bin_report_and_gated_cleanup() {
    let h = E2eHarness::new();
    let report = h.run(&["--json", "bin", "report"]);
    assert_not_still_unimplemented(&report);
    assert_eq!(
        report.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&report)
    );

    let dry = h.run(&["--json", "bin", "cleanup", "--mode", "dry-run"]);
    assert_not_still_unimplemented(&dry);
    assert_eq!(
        dry.status.code(),
        Some(POLICY_GATE as i32),
        "{}",
        stderr(&dry)
    );

    let archive = h.run(&["--json", "bin", "cleanup", "--mode", "archive"]);
    assert_not_still_unimplemented(&archive);
    assert_eq!(
        archive.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stderr(&archive)
    );
}

#[test]
fn e2e_session_queries_remain_available_after_lifecycle() {
    let h = E2eHarness::new();
    let _ = h.run(&["--json", "build", "demo"]);
    let list = h.run(&["--json", "session", "list"]);
    assert_eq!(
        list.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&list)
    );
    let _ = stdout(&list);
}

#[test]
fn e2e_optional_rtk_and_herdr_absence_does_not_block_mvp_skeleton() {
    let h = E2eHarness::new();
    // No rtk/herdr binaries on PATH; doctor/list must remain implemented.
    let out = h.run(&["--json", "tools"]);
    assert_ne!(out.status.code(), Some(NOT_IMPLEMENTED as i32));
}

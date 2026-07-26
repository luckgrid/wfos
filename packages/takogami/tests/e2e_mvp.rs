//! E09.S7 Phase 0 — hermetic MVP acceptance skeleton (review-corrected).
//! Canonical Ontarch package path; plan-aligned payloads; fixture immutability.

#[path = "support/mod.rs"]
mod support;

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use support::{
    copy_dir, hash_tree, sample_cleanup_plan, sample_inventory, write_canonical_fake_ontarch,
    write_executable, write_marker_exe,
};
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
    #[allow(dead_code)]
    root: PathBuf,
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    path_dir: PathBuf,
    #[allow(dead_code)]
    marker: PathBuf,
    path_decoy_marker: PathBuf,
    tracked_hash: String,
}

impl E2eHarness {
    fn new() -> Self {
        let tracked_hash = hash_tree(&e2e_root());
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("e2e");
        fs::create_dir_all(&root).unwrap();
        copy_dir(&e2e_root(), &root);

        // WfOS-like workspace with canonical packages/ontarch.
        let workspace = root.join("workspace");
        let ontarch_pkg = workspace.join("packages/ontarch");
        let registry = ontarch_pkg.join("registry");
        fs::create_dir_all(ontarch_pkg.join("bin")).unwrap();
        fs::create_dir_all(&registry).unwrap();
        copy_dir(&resolution_root().join("registry"), &registry);
        if resolution_root().join("demo").is_dir() {
            copy_dir(&resolution_root().join("demo"), &workspace.join("demo"));
        }
        // Overlay tracked e2e graph (with registry_generation) if present.
        let tracked_graph = root.join("ontarch/registry/graph.json");
        if tracked_graph.is_file() {
            fs::copy(&tracked_graph, registry.join("graph.json")).unwrap();
        }
        if root.join("ontarch/registry/graph.dot").is_file() {
            fs::copy(
                root.join("ontarch/registry/graph.dot"),
                registry.join("graph.dot"),
            )
            .unwrap();
        }

        let state_home = root.join("state");
        fs::create_dir_all(&state_home).unwrap();
        let path_dir = root.join("tools");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["cargo", "rustc", "moon", "demo-bin", "rg", "git"] {
            write_executable(&path_dir.join(name), "#!/bin/sh\nexit 0\n");
        }
        let path_decoy_marker = root.join("MARKER_PATH_DECOY");
        write_marker_exe(&path_dir.join("ontarch"), &path_decoy_marker);

        let marker = root.join("MARKER_CANONICAL");
        let panoply = root.join("PANOPLY_SEEN");
        let inv = sample_inventory(workspace.to_str().unwrap());
        let clean = sample_cleanup_plan("report-only");
        write_canonical_fake_ontarch(
            &ontarch_pkg.join("bin/ontarch"),
            &marker,
            &panoply,
            &inv,
            &clean,
        );

        Self {
            temp,
            root,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
            path_decoy_marker,
            tracked_hash,
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

    fn assert_tracked_unchanged(&self) {
        assert_eq!(
            hash_tree(&e2e_root()),
            self.tracked_hash,
            "tracked e2e fixture tree must remain byte-identical"
        );
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
fn e2e_discovery_and_lifecycle_still_work_on_overlay() {
    let h = E2eHarness::new();
    let tools = h.run(&["--json", "tools"]);
    assert_ne!(tools.status.code(), Some(NOT_IMPLEMENTED as i32));
    assert_eq!(
        tools.status.code(),
        Some(SUCCESS as i32),
        "tools must succeed on overlay: {}",
        stderr(&tools)
    );
    let build = h.run(&["--json", "build", "demo"]);
    assert_ne!(build.status.code(), Some(NOT_IMPLEMENTED as i32));
    assert_eq!(
        build.status.code(),
        Some(SUCCESS as i32),
        "plan-only build must succeed: {}",
        stderr(&build)
    );
    h.assert_tracked_unchanged();
}

#[test]
fn e2e_mvp_path_includes_graph_projection() {
    let h = E2eHarness::new();
    let out = h.run(&["--json", "graph", "--format", "text"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(h.load_records().is_empty(), "graph creates no records");
    assert!(!h.path_decoy_marker.exists());
    h.assert_tracked_unchanged();
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
    assert_eq!(h.load_records().len(), 1);

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
    let body = stdout(&archive);
    assert!(
        body.contains("deferred_unavailable") || stderr(&archive).contains("deferred_unavailable")
    );
    assert!(!h.path_decoy_marker.exists());
    h.assert_tracked_unchanged();
}

#[test]
fn e2e_session_queries_remain_available_after_lifecycle() {
    let h = E2eHarness::new();
    let build = h.run(&["--json", "build", "demo"]);
    assert_eq!(
        build.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&build)
    );
    assert!(
        !h.load_records().is_empty(),
        "lifecycle must produce a command record"
    );
    let list = h.run(&["--json", "session", "list"]);
    assert_eq!(
        list.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&list)
    );
    let _ = stdout(&list);
    h.assert_tracked_unchanged();
}

#[test]
fn e2e_optional_rtk_and_herdr_absence_does_not_block_mvp_skeleton() {
    let h = E2eHarness::new();
    let out = h.run(&["--json", "tools"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_tracked_unchanged();
}

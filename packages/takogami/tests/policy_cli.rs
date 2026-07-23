//! Policy decision CLI matrix (hermetic; no real process spawn).

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use takogami::contracts::{RegistryGeneration, fingerprint_file};
use takogami::exit_codes::{CONTRACT, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, SUCCESS};

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

struct Harness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    workspace: PathBuf,
    registry: PathBuf,
    path_dir: PathBuf,
    marker: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        let registry = workspace.join("registry");
        fs::create_dir_all(&workspace).unwrap();
        copy_dir(&fixture_root(), &workspace);

        let path_dir = workspace.join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        let marker = workspace.join("MARKER_RAN");
        write_marker_exe(&path_dir.join("moon"), &marker);
        write_marker_exe(&path_dir.join("demo-bin"), &marker);
        write_marker_exe(&path_dir.join("rg"), &marker);
        write_marker_exe(&path_dir.join("git"), &marker);
        write_marker_exe(&path_dir.join("pass"), &marker);

        let mut h = Self {
            temp,
            workspace: workspace.clone(),
            registry,
            path_dir,
            marker,
        };
        h.write_hit_units();
        h
    }

    fn write_hit_units(&mut self) {
        let descs = self
            .registry
            .join("sources/descriptors")
            .read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect::<Vec<_>>();

        let mut fps = Vec::new();
        let mut units = Vec::new();
        for path in &descs {
            let rel = format!(
                "registry/sources/descriptors/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            let abs = self.workspace.join(&rel);
            let fp = fingerprint_file(&abs, &rel).unwrap();
            fps.push(fp);
            let text = fs::read_to_string(path).unwrap();
            let authored: toml::Value = toml::from_str(&text).unwrap();
            let id = authored["id"].as_str().unwrap().to_string();
            let entrypoints = authored
                .get("entrypoints")
                .cloned()
                .unwrap_or(toml::Value::Table(Default::default()));
            let entrypoints_json: Value = serde_json::to_value(&entrypoints).unwrap();
            let native = authored
                .get("native")
                .and_then(|n| n.get("manifests"))
                .cloned()
                .unwrap_or(toml::Value::Array(vec![]));
            let native_json: Value = serde_json::to_value(&native).unwrap();
            let root = authored
                .get("paths")
                .and_then(|p| p.get("root"))
                .and_then(|v| v.as_str())
                .unwrap_or("demo");
            units.push(serde_json::json!({
                "id": id,
                "kind": "package",
                "title": id,
                "status": "active",
                "path": root,
                "native_manifests": native_json,
                "entrypoints": entrypoints_json,
                "source": "central",
                "provides": [],
                "requires": [],
            }));
        }

        let meta = RegistryGeneration {
            generated_at: "2026-07-21T00:00:00Z".into(),
            source_fingerprints: fps,
        };
        let doc = serde_json::json!({
            "generated_at": meta.generated_at,
            "registry_generation": meta,
            "summary": {"total": units.len()},
            "units": units,
        });
        fs::write(
            self.registry.join("units.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = bin();
        cmd.args(args)
            .env("TAKOGAMI_ONTARCH_REGISTRY", &self.registry)
            .env("TAKOGAMI_WORKSPACE_ROOT", &self.workspace)
            .env("PATH", &self.path_dir)
            .env_remove("TAKOGAMI_PROFILE")
            .env("SECRET_SENTINEL", "do-not-leak")
            .env("HERDR_SOCKET_PATH", "/tmp/herdr-should-not-appear.sock");
        cmd.output().expect("spawn")
    }

    fn assert_marker_untouched(&self) {
        assert!(!self.marker.exists(), "marker executable must never run");
    }

    fn assert_no_state_home(&self, state: &Path) {
        assert!(!state.exists(), "state home must not be created in S5");
    }
}

fn write_marker_exe(path: &Path, marker: &Path) {
    let script = format!("#!/bin/sh\necho ran >> {}\nexit 0\n", marker.display());
    fs::write(path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
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

#[test]
fn allow_plan_only_includes_policy_decision() {
    let h = Harness::new();
    let state = h.workspace.join("state-home-should-not-exist");
    let out = h.run(&[
        "--json",
        "--state-home",
        state.to_str().unwrap(),
        "build",
        "demo",
    ]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["policy_decision"]["outcome"], "allow");
    assert_eq!(v["data"]["execution_authorized"], true);
    assert_eq!(v["data"]["mode"], "plan_only");
    h.assert_marker_untouched();
    h.assert_no_state_home(&state);
}

#[test]
fn allow_explain_includes_policy_section() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "demo", "--explain"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["policy_decision"]["outcome"], "allow");
    assert_eq!(v["explanation"]["policy"]["request"]["decision"], "allow");
    assert_eq!(v["explanation"]["policy"]["child"]["decision"], "allow");
    assert_eq!(
        v["explanation"]["policy"]["approval_transport"],
        "unavailable"
    );
    h.assert_marker_untouched();
}

#[test]
fn allow_execute_reaches_unavailable_seam() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "demo", "--execute"]);
    assert_eq!(out.status.code(), Some(NOT_IMPLEMENTED as i32));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "execution_unavailable");
    assert_eq!(v["data"]["policy_decision"]["outcome"], "allow");
    h.assert_marker_untouched();
}

#[test]
fn denied_interactive_returns_policy_before_class() {
    let h = Harness::new();
    // Restrictive alt profile blocks demo-bin → policy deny wins over class unavailable.
    let out = h.run(&[
        "--json",
        "--profile",
        "alt-profile",
        "build",
        "interactive-demo",
    ]);
    assert_eq!(
        out.status.code(),
        Some(POLICY_DENY as i32),
        "{}",
        stderr(&out)
    );
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "policy_deny");
    assert!(v["session_id"].as_str().is_some());
    assert!(v["data"]["plan_digest"].as_str().is_some());
    h.assert_marker_untouched();
}

#[test]
fn malformed_policy_is_contract_error() {
    let h = Harness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("policies.json")).unwrap())
            .unwrap();
    let policies = doc["policies"].as_array_mut().unwrap();
    let tak = policies
        .iter_mut()
        .find(|p| p["id"] == "takogami.agent")
        .expect("takogami.agent");
    tak["allow"] = serde_json::json!({"command": ["takogami build"]}); // misspelled field
    fs::write(
        h.registry.join("policies.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert!(
        v["diagnostics"][0]["code"]
            .as_str()
            .unwrap()
            .starts_with("policy_")
    );
    assert!(v["data"]["policy_decision"].is_null() || v["data"].get("policy_decision").is_none());
    h.assert_marker_untouched();
}

#[test]
fn json_gate_deny_is_single_envelope() {
    let mut h = Harness::new();
    // Patch demo entrypoint to a gated child: ontarch bin-cleanup --mode dry-run
    let path = h.registry.join("sources/descriptors/demo.descriptor.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        r#"program = "moon"
args = ["run", "demo:build"]
cwd = "demo"
env_keys = ["PATH"]
backend = "moon"
adapter = "moon-task""#,
        r#"program = "ontarch"
args = ["bin-cleanup", "--mode", "dry-run"]
cwd = "demo"
env_keys = ["PATH"]
backend = "native"
adapter = "direct""#,
    );
    fs::write(&path, text).unwrap();
    h.write_hit_units();
    write_marker_exe(&h.path_dir.join("ontarch"), &h.marker);

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(
        out.status.code(),
        Some(POLICY_GATE as i32),
        "{}",
        stderr(&out)
    );
    let v = parse_json(&out);
    assert_eq!(v["exit_code"], POLICY_GATE);
    assert_eq!(v["diagnostics"][0]["code"], "policy_gate");
    assert_eq!(v["data"]["policy_decision"]["outcome"], "gate");
    assert!(stderr(&out).is_empty() || !stderr(&out).contains('{'));
    h.assert_marker_untouched();
}

#[test]
fn redaction_omits_secrets_and_env() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "demo", "--explain"]);
    let text = stdout(&out);
    assert!(!text.contains("do-not-leak"));
    assert!(!text.contains("herdr-should-not-appear"));
    assert!(!text.contains("SECRET_SENTINEL"));
}

#[test]
fn human_allow_summary_mentions_policy() {
    let h = Harness::new();
    let out = h.run(&["build", "demo"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Policy:"), "{text}");
    assert!(text.contains("Plan only"), "{text}");
    h.assert_marker_untouched();
}

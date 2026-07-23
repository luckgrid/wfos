//! Lifecycle resolution CLI matrix (hermetic fixtures).

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use takogami::contracts::{RegistryGeneration, fingerprint_file};
use takogami::exit_codes::{NOT_IMPLEMENTED, RESOLUTION, SUCCESS};

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
        // Marker-writing fake moon / demo-bin / rg — writes marker if executed.
        write_marker_exe(&path_dir.join("moon"), &marker);
        write_marker_exe(&path_dir.join("demo-bin"), &marker);
        write_marker_exe(&path_dir.join("rg"), &marker);

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
            // Convert TOML entrypoints table to JSON via serde roundtrip.
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

    fn run_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        let mut cmd = bin();
        cmd.args(args)
            .env("TAKOGAMI_ONTARCH_REGISTRY", &self.registry)
            .env("TAKOGAMI_WORKSPACE_ROOT", &self.workspace)
            .env("PATH", &self.path_dir)
            .env("SECRET_SENTINEL", "do-not-leak")
            .env("HERDR_SOCKET_PATH", "/tmp/herdr-should-not-appear.sock");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn")
    }

    fn assert_marker_untouched(&self) {
        assert!(!self.marker.exists(), "marker executable must never run");
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
fn moon_build_plan_json_hit() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["data"]["mode"], "plan_only");
    assert_eq!(v["data"]["resolved_command"]["program"], "moon");
    assert_eq!(
        v["data"]["resolved_command"]["argv"],
        serde_json::json!(["run", "demo:build"])
    );
    assert_eq!(v["data"]["resolved_command"]["backend"], "moon");
    assert_eq!(v["data"]["resolved_command"]["execution_class"], "direct");
    assert_eq!(v["metrics"]["registry_cache"], "hit");
    assert!(v["session_id"].as_str().unwrap().starts_with("tkg_"));
    assert!(
        v["data"]["plan_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let text = stdout(&out);
    assert!(!text.contains("do-not-leak"));
    assert!(!text.contains("herdr-should-not-appear"));
    h.assert_marker_untouched();
}

#[test]
fn native_and_panoply_and_legacy() {
    let h = Harness::new();

    let native = h.run(&["--json", "build", "native-demo"]);
    assert_eq!(
        native.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&native)
    );
    let nv = parse_json(&native);
    assert_eq!(nv["data"]["resolved_command"]["backend"], "native");
    assert_eq!(
        nv["data"]["resolved_command"]["argv"],
        serde_json::json!(["a;b", "x|y", "$HOME"])
    );

    let pan = h.run(&["--json", "build", "panoply-demo"]);
    assert_eq!(pan.status.code(), Some(SUCCESS as i32), "{}", stderr(&pan));
    let pv = parse_json(&pan);
    assert_eq!(pv["data"]["resolved_command"]["backend"], "panoply");
    assert_eq!(pv["data"]["resolved_command"]["program"], "rg");

    let legacy = h.run(&["--json", "build", "legacy-demo"]);
    assert_eq!(
        legacy.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&legacy)
    );
    let lv = parse_json(&legacy);
    assert_eq!(lv["data"]["resolved_command"]["program"], "moon");
    assert!(
        lv["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "legacy_entrypoint_deprecated")
    );
    h.assert_marker_untouched();
}

#[test]
fn profile_precedence() {
    let h = Harness::new();

    let cli = h.run(&["--json", "--profile", "alt-profile", "build", "demo"]);
    assert_eq!(cli.status.code(), Some(SUCCESS as i32));
    assert_eq!(
        parse_json(&cli)["data"]["resolved_command"]["profile_id"],
        "alt-profile"
    );

    let env = h.run_env(
        &["--json", "build", "demo"],
        &[("TAKOGAMI_PROFILE", "alt-profile")],
    );
    assert_eq!(env.status.code(), Some(SUCCESS as i32));
    assert_eq!(
        parse_json(&env)["data"]["resolved_command"]["profile_id"],
        "alt-profile"
    );

    let def = h.run(&["--json", "build", "demo"]);
    assert_eq!(
        parse_json(&def)["data"]["resolved_command"]["profile_id"],
        "workspace-dev"
    );
}

#[test]
fn explain_human_field_order() {
    let h = Harness::new();
    let out = h.run(&["build", "demo", "--explain"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let text = stdout(&out);
    let session_pos = text.find("Session:").unwrap();
    let unit_pos = text.find("Unit:").unwrap();
    let digest_pos = text.find("Plan digest:").unwrap();
    assert!(text.contains("Plan only — no process started"));
    assert!(session_pos < unit_pos && unit_pos < digest_pos);
    h.assert_marker_untouched();
}

#[test]
fn explain_json_reports_safe_executable_provenance() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "demo", "--explain"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let value = parse_json(&out);
    assert_eq!(
        value["explanation"]["command"]["executable"]["selection_source"],
        "path"
    );
    assert_eq!(
        value["explanation"]["command"]["executable"]["path_index"],
        0
    );
    assert!(
        value["explanation"]["command"]["executable"]
            .get("display_path")
            .is_some()
    );
    assert!(!stdout(&out).contains("PATH="));
    h.assert_marker_untouched();
}

#[test]
fn execute_returns_execution_unavailable() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "demo", "--execute"]);
    assert_eq!(out.status.code(), Some(NOT_IMPLEMENTED as i32));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "execution_unavailable");
    assert!(v["session_id"].as_str().is_some());
    assert!(v["data"]["plan_digest"].as_str().is_some());
    h.assert_marker_untouched();
}

#[test]
fn interactive_returns_execution_class_unavailable() {
    let h = Harness::new();
    for unit in ["interactive-demo", "tmux-demo"] {
        let out = h.run(&["--json", "build", unit, "--explain"]);
        assert_eq!(out.status.code(), Some(NOT_IMPLEMENTED as i32), "{unit}");
        let v = parse_json(&out);
        assert_eq!(v["diagnostics"][0]["code"], "execution_class_unavailable");
        assert!(v["explanation"].is_object());
    }
    h.assert_marker_untouched();
}

#[test]
fn unit_not_found_and_session_id() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "no-such-unit"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "unit_not_found");
    assert!(v["session_id"].as_str().unwrap().starts_with("tkg_"));
    assert_eq!(
        v["explanation"]["completed_steps"],
        serde_json::json!(["correlation_id", "registry"])
    );
    assert_eq!(v["explanation"]["freshness"]["registry_cache"], "hit");
    assert!(v["explanation"].get("plan_digest").is_none());
}

#[test]
fn stale_uses_authored_toml() {
    let h = Harness::new();
    // Change authored build task in demo descriptor; units.json still has old routing.
    let path = h.registry.join("sources/descriptors/demo.descriptor.toml");
    let mut text = fs::read_to_string(&path).unwrap();
    text = text.replace("demo:build", "demo:stale-task");
    fs::write(&path, text).unwrap();
    // Fingerprints no longer match → stale
    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["metrics"]["registry_cache"], "stale");
    assert_eq!(
        v["data"]["resolved_command"]["argv"],
        serde_json::json!(["run", "demo:stale-task"])
    );
    h.assert_marker_untouched();
}

#[test]
fn miss_resolves_from_authored() {
    let h = Harness::new();
    fs::remove_file(h.registry.join("units.json")).unwrap();
    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["metrics"]["registry_cache"], "miss");
    assert_eq!(v["data"]["resolved_command"]["program"], "moon");
    h.assert_marker_untouched();
}

#[test]
fn state_home_absent_stays_absent() {
    let h = Harness::new();
    let state = h.temp.path().join("absent-state-home");
    assert!(!state.exists());
    let mut cmd = bin();
    let out = cmd
        .args([
            "--json",
            "--state-home",
            state.to_str().unwrap(),
            "build",
            "demo",
        ])
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &h.workspace)
        .env("PATH", &h.path_dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(!state.exists(), "S4 must not create state-home");
    h.assert_marker_untouched();
}

#[test]
fn unsafe_legacy_and_missing_entrypoint() {
    let h = Harness::new();
    // Inject unsafe legacy unit into units.json
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    // Write a bad legacy descriptor
    let bad = h
        .registry
        .join("sources/descriptors/bad-legacy.descriptor.toml");
    fs::write(
        &bad,
        r#"
id = "bad-legacy"
kind = "package"
title = "bad"
status = "active"
[paths]
root = "demo"
[native]
manifests = ["moon.yml"]
[entrypoints]
build = "echo hi | cat"
"#,
    )
    .unwrap();
    let rel = "registry/sources/descriptors/bad-legacy.descriptor.toml";
    let fp = fingerprint_file(&h.workspace.join(rel), rel).unwrap();
    doc["registry_generation"]["source_fingerprints"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&fp).unwrap());
    doc["units"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "bad-legacy",
            "kind": "package",
            "path": "demo",
            "native_manifests": ["moon.yml"],
            "entrypoints": {"build": "echo hi | cat"},
            "source": "central",
            "provides": [],
            "requires": []
        }));
    // unit with missing check verb
    doc["units"].as_array_mut().unwrap().push(serde_json::json!({
        "id": "no-check",
        "kind": "package",
        "path": "demo",
        "native_manifests": ["Cargo.toml"],
        "entrypoints": {"build": {"program": "demo-bin", "args": [], "backend": "native", "adapter": "direct", "source_manifests": ["Cargo.toml"], "required_policies": ["panoply.agent"]}},
        "source": "central",
        "provides": [],
        "requires": []
    }));
    // fingerprint for no-check — reuse demo descriptor path by writing one
    let no_check_desc = h
        .registry
        .join("sources/descriptors/no-check.descriptor.toml");
    fs::write(
        &no_check_desc,
        r#"
id = "no-check"
kind = "package"
title = "no-check"
status = "active"
[paths]
root = "demo"
[native]
manifests = ["Cargo.toml"]
[entrypoints.build]
program = "demo-bin"
args = []
backend = "native"
adapter = "direct"
source_manifests = ["Cargo.toml"]
required_policies = ["panoply.agent"]
"#,
    )
    .unwrap();
    let rel2 = "registry/sources/descriptors/no-check.descriptor.toml";
    let fp2 = fingerprint_file(&h.workspace.join(rel2), rel2).unwrap();
    doc["registry_generation"]["source_fingerprints"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(&fp2).unwrap());
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let unsafe_out = h.run(&["--json", "build", "bad-legacy"]);
    assert_eq!(unsafe_out.status.code(), Some(RESOLUTION as i32));
    assert_eq!(
        parse_json(&unsafe_out)["diagnostics"][0]["code"],
        "unsafe_legacy_entrypoint"
    );

    let missing = h.run(&["--json", "check", "no-check"]);
    assert_eq!(missing.status.code(), Some(RESOLUTION as i32));
    assert_eq!(
        parse_json(&missing)["diagnostics"][0]["code"],
        "missing_entrypoint"
    );
    h.assert_marker_untouched();
}

#[test]
fn json_purity_on_failure() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "missing-unit-xyz"]);
    let _ = parse_json(&out); // entire stdout is one document
    assert!(stderr(&out).is_empty() || !stderr(&out).contains('{'));
}

#[test]
fn structured_metachar_argv_preserved_literally() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "native-demo"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let argv = &parse_json(&out)["data"]["resolved_command"]["argv"];
    assert_eq!(argv, &serde_json::json!(["a;b", "x|y", "$HOME"]));
    let text = stdout(&out);
    assert!(!text.contains("do-not-leak"));
    h.assert_marker_untouched();
}

#[test]
fn hit_provisional_returns_routing_incomplete() {
    let h = Harness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    let units = doc["units"].as_array_mut().unwrap();
    let demo = units
        .iter_mut()
        .find(|u| u["id"] == "demo")
        .expect("demo unit");
    demo["provisional"] = serde_json::json!(true);
    demo["routing_complete"] = serde_json::json!(false);
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        parse_json(&out)["diagnostics"][0]["code"],
        "routing_incomplete"
    );
    h.assert_marker_untouched();
}

#[test]
fn profile_required_when_no_default() {
    let h = Harness::new();
    fs::write(
        h.registry.join("profiles.json"),
        r#"{"generated_at":"2026-07-21T00:00:00Z","profiles":[{"id":"alt-profile","title":"Alt","purpose":"test","rails":"panoply.agent","rails_bin":"agent-bin","isolation_mode":"branch","isolation_jj":"opt-in","session_state_home":null}]}"#,
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        parse_json(&out)["diagnostics"][0]["code"],
        "profile_required"
    );
    h.assert_marker_untouched();
}

#[test]
fn policy_not_found_fail_closed() {
    let h = Harness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    let units = doc["units"].as_array_mut().unwrap();
    let demo = units
        .iter_mut()
        .find(|u| u["id"] == "demo")
        .expect("demo unit");
    demo["entrypoints"]["build"]["required_policies"] =
        serde_json::json!(["panoply.agent", "missing-policy-xyz"]);
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        parse_json(&out)["diagnostics"][0]["code"],
        "policy_not_found"
    );
    h.assert_marker_untouched();
}

#[test]
fn execute_after_resolution_failure_is_exit_4() {
    let h = Harness::new();
    let out = h.run(&["--json", "build", "no-such-unit", "--execute"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "unit_not_found");
    assert_ne!(v["diagnostics"][0]["code"], "execution_unavailable");
    h.assert_marker_untouched();
}

#[test]
fn invalid_cwd_escapes_workspace() {
    let h = Harness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    let units = doc["units"].as_array_mut().unwrap();
    let native = units
        .iter_mut()
        .find(|u| u["id"] == "native-demo")
        .expect("native-demo");
    native["entrypoints"]["build"]["cwd"] = serde_json::json!("/tmp");
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "native-demo"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    assert_eq!(parse_json(&out)["diagnostics"][0]["code"], "invalid_cwd");
    h.assert_marker_untouched();
}

#[test]
fn unsupported_backend_pair() {
    let h = Harness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    let units = doc["units"].as_array_mut().unwrap();
    let native = units
        .iter_mut()
        .find(|u| u["id"] == "native-demo")
        .expect("native-demo");
    native["entrypoints"]["build"]["backend"] = serde_json::json!("moon");
    native["entrypoints"]["build"]["adapter"] = serde_json::json!("direct");
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "native-demo"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        parse_json(&out)["diagnostics"][0]["code"],
        "unsupported_backend"
    );
    h.assert_marker_untouched();
}

#[test]
fn human_failure_renders_same_partial_trace() {
    let h = Harness::new();
    let out = h.run(&["--no-color", "build", "no-such-unit"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    assert!(stderr(&out).contains("Resolution trace:"));
    assert!(stderr(&out).contains("Completed steps: correlation_id, registry"));
    assert!(stdout(&out).is_empty());
    h.assert_marker_untouched();
}

#[test]
fn malformed_requested_authored_descriptor_fails_closed() {
    let h = Harness::new();
    let descriptor = h.registry.join("sources/descriptors/demo.descriptor.toml");
    fs::write(
        &descriptor,
        "id = \"demo\"\n[entrypoints.build\nprogram = \"moon\"\n",
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    let value = parse_json(&out);
    assert_eq!(value["diagnostics"][0]["code"], "invalid_descriptor");
    assert_eq!(
        value["explanation"]["completed_steps"],
        serde_json::json!(["correlation_id", "registry", "unit"])
    );
    assert!(!stdout(&out).contains(h.temp.path().to_str().unwrap()));
    h.assert_marker_untouched();
}

#[test]
fn malformed_unknown_identity_descriptor_cannot_hide_duplicate() {
    let h = Harness::new();
    fs::write(
        h.registry
            .join("sources/descriptors/unknown-malformed.descriptor.toml"),
        "this is not toml and has no attributable id",
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    assert_eq!(
        parse_json(&out)["diagnostics"][0]["code"],
        "invalid_descriptor"
    );
    h.assert_marker_untouched();
}

#[test]
fn duplicate_valid_authored_descriptors_are_ambiguous() {
    let h = Harness::new();
    let original = h.registry.join("sources/descriptors/demo.descriptor.toml");
    let duplicate = h
        .registry
        .join("sources/descriptors/demo-duplicate.descriptor.toml");
    fs::copy(original, duplicate).unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    let value = parse_json(&out);
    assert_eq!(value["diagnostics"][0]["code"], "descriptor_ambiguous");
    assert_eq!(
        value["explanation"]["completed_steps"],
        serde_json::json!(["correlation_id", "registry", "unit"])
    );
    h.assert_marker_untouched();
}

#[test]
fn structured_entrypoint_unknown_field_is_rejected() {
    let h = Harness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    let demo = doc["units"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|unit| unit["id"] == "demo")
        .unwrap();
    demo["entrypoints"]["build"]["arg"] = serde_json::json!(["misspelled"]);
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "demo"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    assert_eq!(
        parse_json(&out)["diagnostics"][0]["code"],
        "registry_unavailable"
    );
    h.assert_marker_untouched();
}

#[test]
fn same_basename_manifest_substitution_is_rejected() {
    let h = Harness::new();
    fs::create_dir_all(h.workspace.join("demo/a")).unwrap();
    fs::create_dir_all(h.workspace.join("demo/b")).unwrap();
    fs::write(
        h.workspace.join("demo/a/Cargo.toml"),
        "[package]\nname='a'\n",
    )
    .unwrap();
    fs::write(
        h.workspace.join("demo/b/Cargo.toml"),
        "[package]\nname='b'\n",
    )
    .unwrap();

    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    let native = doc["units"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|unit| unit["id"] == "native-demo")
        .unwrap();
    native["native_manifests"] = serde_json::json!(["a/Cargo.toml"]);
    native["entrypoints"]["build"]["source_manifests"] = serde_json::json!(["b/Cargo.toml"]);
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "native-demo"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    let value = parse_json(&out);
    assert_eq!(value["diagnostics"][0]["code"], "missing_manifest");
    assert!(
        value["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("not authorized")
    );
    h.assert_marker_untouched();
}

#[test]
fn distinct_panoply_detect_candidates_are_ambiguous() {
    let h = Harness::new();
    let first = h.path_dir.join("rg-first");
    let second = h.path_dir.join("rg-second");
    write_marker_exe(&first, &h.marker);
    write_marker_exe(&second, &h.marker);
    let tools = serde_json::json!({
        "generated_at": "2026-07-22T00:00:00Z",
        "summary": {"total": 2},
        "tools": [
            {"id": "rg", "installed": true, "detect": first},
            {"id": "rg", "installed": true, "detect": second}
        ]
    });
    fs::write(
        h.registry.join("tools.json"),
        serde_json::to_string_pretty(&tools).unwrap(),
    )
    .unwrap();

    let out = h.run(&["--json", "build", "panoply-demo"]);
    assert_eq!(out.status.code(), Some(RESOLUTION as i32));
    let value = parse_json(&out);
    assert_eq!(value["diagnostics"][0]["code"], "executable_ambiguous");
    assert_eq!(
        value["explanation"]["completed_steps"],
        serde_json::json!([
            "correlation_id",
            "registry",
            "unit",
            "descriptor",
            "entrypoint",
            "cwd",
            "manifests",
            "backend"
        ])
    );
    assert!(!stdout(&out).contains("PATH="));
    h.assert_marker_untouched();
}

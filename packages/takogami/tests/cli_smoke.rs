use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use takogami::contracts::{
    RegistryGeneration, SourceFingerprint, fingerprint_bytes, fingerprint_file,
};
use takogami::exit_codes::{INTERNAL, NOT_IMPLEMENTED, SUCCESS, USAGE};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_takogami"))
}

fn run(args: &[&str]) -> Output {
    bin().args(args).output().expect("failed to spawn takogami")
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("stdout utf-8")
}

fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).expect("stderr utf-8")
}

fn run_with_env(
    args: &[&str],
    registry: &Path,
    workspace: &Path,
    path_prepend: Option<&Path>,
) -> Output {
    let mut cmd = bin();
    cmd.args(args)
        .env("TAKOGAMI_ONTARCH_REGISTRY", registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", workspace);
    if let Some(prepend) = path_prepend {
        // Isolate from host PATH so hermetic required-tool checks are deterministic.
        cmd.env("PATH", prepend.display().to_string());
    }
    cmd.output().expect("spawn")
}

fn write_units(
    registry: &Path,
    workspace: &Path,
    units_json: &str,
    source_rel: &str,
    source_bytes: &[u8],
) {
    fs::create_dir_all(registry).unwrap();
    let src = workspace.join(source_rel);
    if let Some(parent) = src.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&src, source_bytes).unwrap();
    let fp = fingerprint_file(&src, source_rel).unwrap();
    let meta = RegistryGeneration {
        generated_at: "2026-07-20T00:00:00Z".into(),
        source_fingerprints: vec![fp],
    };
    let mut doc: Value = serde_json::from_str(units_json).unwrap();
    doc["registry_generation"] = serde_json::to_value(&meta).unwrap();
    doc["generated_at"] = Value::String(meta.generated_at.clone());
    fs::write(
        registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
}

fn fake_tool_dir() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    for name in ["cargo", "rustc", "moon"] {
        let path = temp.path().join(name);
        fs::write(&path, b"").expect("write fake tool");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    temp
}

#[test]
fn version_reports_package_version() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("takogami 0.1.0"));
}

#[test]
fn help_lists_mvp_commands_and_globals() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = stdout(&output);
    for needle in [
        "scan",
        "list",
        "info",
        "doctor",
        "tools",
        "interfaces",
        "dev",
        "build",
        "check",
        "graph",
        "bin",
        "session",
        "--json",
        "--profile",
        "--state-home",
        "--no-color",
        "--verbose",
    ] {
        assert!(help.contains(needle), "missing `{needle}` in help");
    }
    assert!(!help.contains("workstream"));
}

#[test]
fn build_is_lifecycle_verb_not_workstream_namespace() {
    let output = run(&["build", "--help"]);
    assert!(output.status.success());
    let help = stdout(&output);
    assert!(help.contains("<UNIT>"));
    assert!(!help.contains("workstream"));
}

#[test]
fn unknown_command_fails_without_panic() {
    let output = run(&["definitely-not-a-command"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(i32::from(USAGE)));
}

#[test]
fn session_list_still_not_implemented() {
    let output = run(&["session", "list", "--json"]);
    assert_eq!(output.status.code(), Some(i32::from(NOT_IMPLEMENTED)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).expect("json");
    assert_eq!(value["command"], "session");
    assert_eq!(value["diagnostics"][0]["code"], "not_implemented");
}

#[test]
fn doctor_json_reports_controller_readiness() {
    let tools = fake_tool_dir();
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "desc.toml",
        b"id = \"x\"\n",
    );
    let state = temp.path().join("state");
    let output = run_with_env(
        &["doctor", "--json", "--state-home", state.to_str().unwrap()],
        &registry,
        &workspace,
        Some(tools.path()),
    );
    assert_eq!(
        output.status.code(),
        Some(i32::from(SUCCESS)),
        "stderr={}",
        stderr(&output)
    );
    let value: Value = serde_json::from_str(stdout(&output).trim()).expect("json");
    assert_eq!(value["command"], "doctor");
    assert_eq!(value["data"]["scope"], "controller_readiness");
    assert_eq!(value["data"]["session_readiness"], false);
}

#[test]
fn doctor_human_reports_readiness_scope() {
    let tools = fake_tool_dir();
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "desc.toml",
        b"id = \"x\"\n",
    );
    let state = temp.path().join("state");
    let output = run_with_env(
        &["doctor", "--state-home", state.to_str().unwrap()],
        &registry,
        &workspace,
        Some(tools.path()),
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let body = stdout(&output);
    assert!(body.contains("controller readiness"));
    assert!(body.contains("cargo"));
    assert!(body.contains("herdr"));
}

#[test]
fn list_units_hit_json_purity() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":1},"units":[{"id":"demo","kind":"workspace","path":"Build/src/workspaces/demo","entrypoints":{},"native_manifests":[],"provides":[],"requires":[]}]}"#,
        "Build/src/workspaces/wfos/packages/ontarch/descriptors/demo.descriptor.toml",
        b"id = \"demo\"\n",
    );
    let output = run_with_env(&["list", "units", "--json"], &registry, &workspace, None);
    assert_eq!(
        output.status.code(),
        Some(i32::from(SUCCESS)),
        "{}",
        stderr(&output)
    );
    let body = stdout(&output).trim();
    assert!(!body.contains('\n'), "json mode must emit one document");
    let value: Value = serde_json::from_str(body).expect("json");
    assert_eq!(value["schema_version"], "0.1.0");
    assert_eq!(value["command"], "list");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["metrics"]["registry_cache"], "hit");
    assert_eq!(value["data"]["count"], 1);
}

#[test]
fn list_units_missing_registry() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    fs::create_dir_all(&registry).unwrap();
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    // source fallback descriptor
    let desc_dir = registry.join("sources/descriptors");
    fs::create_dir_all(&desc_dir).unwrap();
    fs::write(
        desc_dir.join("fallback.descriptor.toml"),
        b"id = \"fallback\"\nkind = \"workspace\"\n",
    )
    .unwrap();

    let output = run_with_env(&["list", "units", "--json"], &registry, &workspace, None);
    assert_eq!(output.status.code(), Some(i32::from(SUCCESS)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["metrics"]["registry_cache"], "miss");
    assert_eq!(value["data"]["units"][0]["id"], "fallback");
    assert_eq!(value["data"]["units"][0]["provisional"], true);
}

#[test]
fn list_units_stale_when_fingerprint_mismatches() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&registry).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    let rel = "src.toml";
    let src = workspace.join(rel);
    fs::write(&src, b"id = \"x\"\n").unwrap();
    let meta = RegistryGeneration {
        generated_at: "t".into(),
        source_fingerprints: vec![SourceFingerprint {
            path: rel.into(),
            algorithm: "sha256".into(),
            digest: fingerprint_bytes(b"other").digest,
        }],
    };
    let doc = serde_json::json!({
        "generated_at": "t",
        "registry_generation": meta,
        "summary": {"total": 1},
        "units": [{"id":"x","kind":"workspace","path":"x","entrypoints":{},"native_manifests":[],"provides":[],"requires":[]}]
    });
    fs::write(registry.join("units.json"), doc.to_string()).unwrap();
    let output = run_with_env(&["list", "units", "--json"], &registry, &workspace, None);
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["metrics"]["registry_cache"], "stale");
}

#[test]
fn list_units_malformed_registry() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    fs::create_dir_all(&registry).unwrap();
    fs::write(registry.join("units.json"), b"{not-json").unwrap();
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let output = run_with_env(&["list", "units", "--json"], &registry, &workspace, None);
    assert_ne!(output.status.code(), Some(i32::from(SUCCESS)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["diagnostics"][0]["code"], "invalid_registry");
}

#[test]
fn info_unknown_unit() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "d.toml",
        b"id=\"z\"\n",
    );
    let output = run_with_env(&["info", "nope", "--json"], &registry, &workspace, None);
    assert_eq!(output.status.code(), Some(i32::from(USAGE)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["diagnostics"][0]["code"], "not_found");
}

#[test]
fn list_invalid_filter() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "d.toml",
        b"id=\"z\"\n",
    );
    let output = run_with_env(
        &["list", "units", "--filter", "bogus=1", "--json"],
        &registry,
        &workspace,
        None,
    );
    assert_eq!(output.status.code(), Some(i32::from(USAGE)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["diagnostics"][0]["code"], "invalid_filter");
}

#[test]
fn scan_descriptor_less_provisional() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "d.toml",
        b"id=\"z\"\n",
    );
    let scan = serde_json::json!({
        "generated_at": "t",
        "registry_generation": {
            "generated_at": "t",
            "source_fingerprints": []
        },
        "root": "/tmp",
        "summary": {"total": 1, "clean": 1, "dirty": 0},
        "workspaces": [{
            "path": "Build/src/workspaces/orphan-ws",
            "kind": "workspace",
            "native_manifests": ["package.json"],
            "lint_check_commands": ["echo never-run"]
        }]
    });
    // empty fingerprints → stale; still discoverable
    fs::write(registry.join("scan.json"), scan.to_string()).unwrap();
    let output = run_with_env(&["scan", "--json"], &registry, &workspace, None);
    assert_eq!(
        output.status.code(),
        Some(i32::from(SUCCESS)),
        "{}",
        stderr(&output)
    );
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["data"]["lint_check_commands_evidence_only"], true);
    assert_eq!(value["data"]["provisional"][0]["id"], "orphan-ws");
    assert_eq!(value["data"]["provisional"][0]["provisional"], true);
    assert_eq!(value["data"]["provisional"][0]["routing_complete"], false);
    assert!(
        value["data"]["provisional"][0]["entrypoints"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn doctor_missing_herdr_optional_still_ready() {
    let tools = fake_tool_dir();
    // deliberately no herdr binary
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "d.toml",
        b"id=\"z\"\n",
    );
    let state = temp.path().join("state");
    let output = run_with_env(
        &["doctor", "--json", "--state-home", state.to_str().unwrap()],
        &registry,
        &workspace,
        Some(tools.path()),
    );
    assert_eq!(output.status.code(), Some(i32::from(SUCCESS)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["data"]["ready"], true);
    let herdr = value["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "herdr")
        .unwrap();
    assert_eq!(herdr["ok"], false);
    assert_eq!(herdr["severity"], "optional");
}

#[test]
fn doctor_fails_without_required_cargo() {
    let temp = tempfile::tempdir().unwrap();
    // empty PATH prepend — no cargo
    let empty = temp.path().join("emptybin");
    fs::create_dir_all(&empty).unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":0},"units":[]}"#,
        "d.toml",
        b"id=\"z\"\n",
    );
    let state = temp.path().join("state");
    let output = run_with_env(
        &["doctor", "--json", "--state-home", state.to_str().unwrap()],
        &registry,
        &workspace,
        Some(&empty),
    );
    assert_eq!(output.status.code(), Some(i32::from(INTERNAL)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["data"]["ready"], false);
}

#[test]
fn ambiguous_unit_id_in_registry() {
    let temp = tempfile::tempdir().unwrap();
    let registry = temp.path().join("registry");
    let workspace = temp.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    write_units(
        &registry,
        &workspace,
        r#"{"summary":{"total":2},"units":[
          {"id":"dup","kind":"workspace","path":"a","entrypoints":{},"native_manifests":[],"provides":[],"requires":[]},
          {"id":"dup","kind":"workspace","path":"b","entrypoints":{},"native_manifests":[],"provides":[],"requires":[]}
        ]}"#,
        "d.toml",
        b"id=\"z\"\n",
    );
    let output = run_with_env(&["info", "dup", "--json"], &registry, &workspace, None);
    assert_eq!(output.status.code(), Some(i32::from(USAGE)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).unwrap();
    assert_eq!(value["diagnostics"][0]["code"], "ambiguous");
}

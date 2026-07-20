use serde_json::Value;
use std::process::{Command, Output};
use takogami::exit_codes::{NOT_IMPLEMENTED, USAGE};

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
    assert!(
        !help.contains("workstream"),
        "help must not advertise workstream namespace in S1"
    );
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
fn unimplemented_scan_json_is_single_parseable_envelope() {
    let output = run(&["scan", "--json"]);
    assert_eq!(output.status.code(), Some(i32::from(NOT_IMPLEMENTED)));
    let body = stdout(&output).trim();
    assert!(!body.is_empty());
    assert!(!body.contains('\n'), "json mode must emit one document");
    let value: Value = serde_json::from_str(body).expect("valid json envelope");
    assert_eq!(value["schema_version"], "0.1.0");
    assert_eq!(value["command"], "scan");
    assert_eq!(value["status"], "error");
    assert_eq!(value["exit_code"], NOT_IMPLEMENTED);
    assert_eq!(value["diagnostics"][0]["code"], "not_implemented");
}

#[test]
fn unimplemented_session_list_json_envelope() {
    let output = run(&["session", "list", "--json"]);
    assert_eq!(output.status.code(), Some(i32::from(NOT_IMPLEMENTED)));
    let value: Value = serde_json::from_str(stdout(&output).trim()).expect("json");
    assert_eq!(value["command"], "session");
    assert_eq!(value["diagnostics"][0]["code"], "not_implemented");
}

fn fake_tool_dir() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    for name in ["cargo", "rustc", "moon"] {
        let path = temp.path().join(name);
        std::fs::write(&path, b"").expect("write fake tool");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    temp
}

fn run_with_path(args: &[&str], prepend: &std::path::Path) -> Output {
    let path = format!("{}:/usr/bin:/bin", prepend.display());
    bin()
        .args(args)
        .env("PATH", path)
        .output()
        .expect("failed to spawn takogami")
}

#[test]
fn doctor_human_reports_skeleton_scope() {
    let temp = fake_tool_dir();
    let output = run_with_path(&["doctor"], temp.path());
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let body = stdout(&output);
    assert!(body.contains("build toolchain skeleton"));
    assert!(body.contains("registry/session/RTK checks arrive in S3"));
    assert!(body.contains("cargo"));
    assert!(body.contains("rustc"));
    assert!(body.contains("moon"));
}

#[test]
fn doctor_json_reports_skeleton_scope() {
    let temp = fake_tool_dir();
    let output = run_with_path(&["doctor", "--json"], temp.path());
    assert!(output.status.success());
    let value: Value = serde_json::from_str(stdout(&output).trim()).expect("json");
    assert_eq!(value["command"], "doctor");
    assert_eq!(value["data"]["scope"], "build_toolchain_skeleton");
    assert_eq!(value["data"]["registry_readiness"], false);
    assert_eq!(value["data"]["session_readiness"], false);
    assert_eq!(value["data"]["rtk_readiness"], false);
}

#[test]
fn doctor_respects_injected_path_for_cargo() {
    let temp = fake_tool_dir();
    let output = run_with_path(&["doctor", "--json"], temp.path());
    let value: Value = serde_json::from_str(stdout(&output).trim()).expect("json");
    let checks = value["data"]["checks"].as_array().expect("checks array");
    let cargo = checks
        .iter()
        .find(|c| c["name"] == "cargo")
        .expect("cargo check");
    assert_eq!(cargo["ok"], true);
}

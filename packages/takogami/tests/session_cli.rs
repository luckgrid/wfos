//! Session list/show/latest CLI tests (hermetic; always uses a temp --state-home).

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use takogami::contracts::types::{
    ExecutionRecord, OutputSummary, PolicyDecision, RECORD_KIND_COMMAND_EXECUTION, RequestRecord,
    RuntimeCommandRecord, SCHEMA_VERSION,
};
use takogami::exit_codes::{SUCCESS, USAGE};

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
    state_home: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        let registry = workspace.join("registry");
        let state_home = temp.path().join("state-home");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state_home).unwrap();
        copy_dir(&fixture_root(), &workspace);
        Self {
            temp,
            workspace,
            registry,
            state_home,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = bin();
        cmd.arg("--state-home")
            .arg(&self.state_home)
            .args(args)
            .env("TAKOGAMI_ONTARCH_REGISTRY", &self.registry)
            .env("TAKOGAMI_WORKSPACE_ROOT", &self.workspace)
            .env("TAKOGAMI_STATE_HOME", &self.state_home)
            .env_remove("TAKOGAMI_PROFILE")
            .env_remove("XDG_STATE_HOME");
        cmd.output().expect("spawn")
    }

    fn write_record(&self, record: &RuntimeCommandRecord) {
        let path = self.state_home.join(format!("{}.json", record.session_id));
        fs::write(path, serde_json::to_string_pretty(record).unwrap()).unwrap();
    }
}

fn sample(
    session_id: &str,
    started_at: &str,
    outcome: &str,
    ended_at: Option<&str>,
) -> RuntimeCommandRecord {
    RuntimeCommandRecord {
        schema_version: SCHEMA_VERSION.into(),
        record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
        session_id: session_id.into(),
        plan_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .into(),
        parent_session_id: None,
        work_session_id: None,
        runtime_context: None,
        started_at: started_at.into(),
        ended_at: ended_at.map(str::to_string),
        actor: "agent".into(),
        profile_id: "workspace-dev".into(),
        request: RequestRecord {
            command: "build".into(),
            unit_id: Some("demo".into()),
            verb: Some("build".into()),
            flags: vec![],
        },
        resolution: None,
        policy_decision: PolicyDecision::Allow {
            matched_rules: vec![],
        },
        execution: ExecutionRecord {
            started: outcome == "completed",
            pid: if outcome == "completed" {
                Some(4242)
            } else {
                None
            },
            exit_code: if outcome == "completed" {
                Some(0)
            } else {
                None
            },
            signal: None,
            outcome: outcome.into(),
        },
        source_fingerprints: vec![],
        output_summary: OutputSummary {
            stdout_bytes: 0,
            stderr_bytes: 0,
            truncated: false,
            encoding: "utf-8".into(),
            compressor: "none".into(),
        },
        error: None,
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
fn session_list_show_latest_with_state_home_fixtures() {
    let h = Harness::new();
    h.write_record(&sample(
        "tkg_older",
        "2026-07-20T10:00:00Z",
        "planned",
        Some("2026-07-20T10:00:01Z"),
    ));
    h.write_record(&sample(
        "tkg_newer",
        "2026-07-21T12:00:00Z",
        "completed",
        Some("2026-07-21T12:00:02Z"),
    ));

    let list = h.run(&["--json", "session", "list"]);
    assert_eq!(
        list.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&list)
    );
    let lv = parse_json(&list);
    assert_eq!(lv["data"]["count"], 2);
    assert_eq!(lv["data"]["records"][0]["session_id"], "tkg_newer");
    assert_eq!(lv["data"]["records"][0]["record_kind"], "command_execution");
    assert_eq!(lv["data"]["records"][1]["session_id"], "tkg_older");

    let show = h.run(&["--json", "session", "show", "tkg_older"]);
    assert_eq!(
        show.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&show)
    );
    let sv = parse_json(&show);
    assert_eq!(sv["data"]["session_id"], "tkg_older");
    assert_eq!(sv["data"]["execution"]["outcome"], "planned");
    assert_eq!(sv["data"]["record_kind"], "command_execution");

    let latest = h.run(&["--json", "session", "latest"]);
    assert_eq!(
        latest.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&latest)
    );
    let latest_v = parse_json(&latest);
    assert_eq!(latest_v["data"]["session_id"], "tkg_newer");
    assert_eq!(latest_v["data"]["execution"]["outcome"], "completed");
}

#[test]
fn session_latest_empty_is_not_found() {
    let h = Harness::new();
    let out = h.run(&["--json", "session", "latest"]);
    assert_eq!(out.status.code(), Some(USAGE as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "not_found");
}

#[test]
fn session_show_invalid_id_is_usage() {
    let h = Harness::new();
    let out = h.run(&["--json", "session", "show", "../escape"]);
    assert_eq!(out.status.code(), Some(USAGE as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "usage");
}

#[test]
fn session_show_absent_is_not_found() {
    let h = Harness::new();
    let out = h.run(&["--json", "session", "show", "tkg_missing"]);
    assert_eq!(out.status.code(), Some(USAGE as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["diagnostics"][0]["code"], "not_found");
}

#[test]
fn human_session_output_labels_record_kind() {
    let h = Harness::new();
    h.write_record(&sample(
        "tkg_human",
        "2026-07-21T09:00:00Z",
        "planned",
        Some("2026-07-21T09:00:01Z"),
    ));

    let list = h.run(&["session", "list"]);
    assert_eq!(
        list.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&list)
    );
    let text = stdout(&list);
    assert!(
        text.contains("Record kind: command_execution"),
        "missing record_kind label: {text}"
    );

    let show = h.run(&["session", "show", "tkg_human"]);
    assert_eq!(
        show.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&show)
    );
    let show_text = stdout(&show);
    assert!(
        show_text.contains("Record kind: command_execution"),
        "{show_text}"
    );

    let latest = h.run(&["session", "latest"]);
    assert_eq!(
        latest.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&latest)
    );
    assert!(
        stdout(&latest).contains("Record kind: command_execution"),
        "{}",
        stdout(&latest)
    );
}

#[test]
fn empty_session_list_is_success() {
    let h = Harness::new();
    let out = h.run(&["--json", "session", "list"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["count"], 0);
    assert_eq!(v["data"]["records"].as_array().unwrap().len(), 0);
}

//! Native execution CLI tests (hermetic; always uses a temp --state-home).

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use takogami::contracts::{RegistryGeneration, fingerprint_file};
use takogami::execution::DEFAULT_LIMIT_BYTES;
use takogami::exit_codes::{EXECUTION_IO, POLICY_DENY, POLICY_GATE, SUCCESS};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_takogami"))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution")
}

fn execution_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/execution")
        .join(name)
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
    state_home: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        let registry = workspace.join("registry");
        let state_home = temp.path().join("state-home");
        fs::create_dir_all(&workspace).unwrap();
        copy_dir(&fixture_root(), &workspace);

        let path_dir = workspace.join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        let marker = workspace.join("MARKER_RAN");
        write_marker_exe(&path_dir.join("moon"), &marker);
        write_marker_exe(&path_dir.join("demo-bin"), &marker);
        write_marker_exe(&path_dir.join("rg"), &marker);
        for name in ["git", "pass", "gh", "ontarch", "mystery", "rm"] {
            write_marker_exe(&path_dir.join(name), &marker);
        }

        let mut h = Self {
            temp,
            workspace: workspace.clone(),
            registry,
            path_dir,
            marker,
            state_home,
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

    fn install_python_child(&self, name: &str, fixture: &str) {
        // Absolute shebang: sealed child env clears PATH, so `#!/usr/bin/env python3` fails.
        let py = python_executable();
        let src = fs::read_to_string(execution_fixture(fixture)).unwrap();
        let body: String = src
            .lines()
            .skip_while(|l| l.starts_with("#!"))
            .collect::<Vec<_>>()
            .join("\n");
        let dest = self.path_dir.join(name);
        fs::write(&dest, format!("#!{}\n{body}\n", py.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn set_demo_build(&mut self, program: &str, args: &[&str], env_keys: &[&str]) {
        let path = self
            .registry
            .join("sources/descriptors/demo.descriptor.toml");
        let text = fs::read_to_string(&path).unwrap();
        let start = text.find("[entrypoints.build]").expect("entrypoints.build");
        let rest = &text[start + "[entrypoints.build]".len()..];
        let end_rel = rest.find("\n[").map(|i| i + 1).unwrap_or(rest.len());
        let end = start + "[entrypoints.build]".len() + end_rel;
        let args_toml = args
            .iter()
            .map(|a| toml_basic_string(a))
            .collect::<Vec<_>>()
            .join(", ");
        let keys_toml = env_keys
            .iter()
            .map(|k| toml_basic_string(k))
            .collect::<Vec<_>>()
            .join(", ");
        let block = format!(
            "[entrypoints.build]\n\
             program = {program}\n\
             args = [{args_toml}]\n\
             cwd = \"demo\"\n\
             env_keys = [{keys_toml}]\n\
             backend = \"native\"\n\
             adapter = \"direct\"\n\
             source_manifests = [\"Cargo.toml\"]\n\
             required_policies = [\"panoply.agent\"]\n",
            program = toml_basic_string(program),
        );
        let mut new_text = String::new();
        new_text.push_str(&text[..start]);
        new_text.push_str(&block);
        if end < text.len() {
            new_text.push_str(&text[end..]);
        }
        fs::write(&path, new_text).unwrap();
        self.write_hit_units();
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
            .env("SECRET_SENTINEL", "do-not-leak")
            .env("HERDR_SOCKET_PATH", "/tmp/herdr-should-not-appear.sock")
            .env("BENIGN_EXTRA", "should-not-reach-child");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn")
    }

    fn assert_marker_untouched(&self) {
        assert!(!self.marker.exists(), "marker executable must never run");
    }

    /// Marks `workspace-dev` as RTK-eligible so human-mode `finalize_output` takes its
    /// combined-stream branch, without requiring a real RTK adapter binary on `PATH`.
    fn enable_rtk_compressor(&self) {
        let path = self.registry.join("profiles.json");
        let mut document: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let profiles = document["profiles"].as_array_mut().unwrap();
        let profile = profiles
            .iter_mut()
            .find(|p| p["id"] == "workspace-dev")
            .unwrap();
        profile["output_compressor"] = Value::String("rtk".into());
        fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();
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

fn python_executable() -> PathBuf {
    let out = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("locate python3");
    assert!(
        out.status.success(),
        "python3 lookup failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    PathBuf::from(String::from_utf8(out.stdout).unwrap().trim())
}

fn toml_basic_string(s: &str) -> String {
    let mut out = String::from('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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

#[test]
fn literal_argv_with_spaces_quotes_and_metacharacters() {
    let mut h = Harness::new();
    h.install_python_child("demo-bin", "print_argv.py");
    let tokens = [
        "space in token",
        "it's",
        "double\"quote",
        "*",
        "?",
        ";",
        "|",
        "&",
        "$(echo hi)",
        "`echo hi`",
        "line\nbreak",
        "--",
        "-leading-dash",
        "café",
    ];
    h.set_demo_build("demo-bin", &tokens, &["PATH"]);

    let out = h.run(&["--json", "build", "demo", "--execute"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_one_json_document(stdout(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["mode"], "executed");
    assert_eq!(v["data"]["execution_requested"], true);
    let child_stdout = v["child"]["stdout"].as_str().expect("child stdout");
    let reported: Value = serde_json::from_str(child_stdout.trim()).unwrap();
    let argv = reported["argv"].as_array().unwrap();
    assert_eq!(argv.len(), tokens.len());
    for (i, want) in tokens.iter().enumerate() {
        assert_eq!(argv[i]["len"], want.len() as u64);
        assert_eq!(argv[i]["value"], *want);
    }
}

#[test]
fn env_clear_only_sealed_keys_reach_child() {
    let mut h = Harness::new();
    h.install_python_child("demo-bin", "print_env.py");
    h.set_demo_build("demo-bin", &[], &["PATH", "BENIGN_SEALED"]);

    let out = h.run_env(
        &["--json", "build", "demo", "--execute"],
        &[("BENIGN_SEALED", "ok-value")],
    );
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    let child_stdout = v["child"]["stdout"].as_str().unwrap();
    let reported: Value = serde_json::from_str(child_stdout.trim()).unwrap();
    let keys: Vec<&str> = reported["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert!(keys.contains(&"PATH"));
    assert!(keys.contains(&"BENIGN_SEALED"));
    assert!(!keys.contains(&"SECRET_SENTINEL"));
    assert!(!keys.contains(&"BENIGN_EXTRA"));
    assert!(!keys.contains(&"HERDR_SOCKET_PATH"));
    assert_eq!(reported["has_secret_sentinel"], false);
    assert_eq!(reported["has_herdr"], false);
    let envelope = stdout(&out);
    assert!(!envelope.contains("ok-value"));
    assert!(!envelope.contains("do-not-leak"));
}

#[test]
fn secret_declared_env_key_fails_before_spawn() {
    let mut h = Harness::new();
    h.install_python_child("demo-bin", "print_env.py");
    h.set_demo_build("demo-bin", &[], &["PATH", "API_TOKEN"]);

    let out = h.run_env(
        &["--json", "build", "demo", "--execute"],
        &[("API_TOKEN", "should-never-be-read")],
    );
    assert_eq!(
        out.status.code(),
        Some(EXECUTION_IO as i32),
        "{}",
        stderr(&out)
    );
    let v = parse_json(&out);
    assert_eq!(v["data"]["mode"], "executed");
    assert_eq!(v["data"]["execution"]["outcome"], "failed_to_spawn");
    assert!(
        v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "execution_contract"
                && d["message"].as_str().unwrap().contains("API_TOKEN")),
        "{v}"
    );
    assert!(!stdout(&out).contains("should-never-be-read"));
    h.assert_marker_untouched();
}

#[test]
fn pending_record_appears_before_marker_or_final_has_pid() {
    let mut h = Harness::new();
    h.install_python_child("demo-bin", "sleep_mark.py");
    let marker = h.marker.clone();
    h.set_demo_build("demo-bin", &["0.5", marker.to_str().unwrap()], &["PATH"]);

    let child = bin()
        .arg("--state-home")
        .arg(&h.state_home)
        .args(["--json", "build", "demo", "--execute"])
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &h.workspace)
        .env("TAKOGAMI_STATE_HOME", &h.state_home)
        .env("PATH", &h.path_dir)
        .env_remove("TAKOGAMI_PROFILE")
        .env_remove("XDG_STATE_HOME")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn takogami");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut saw_pending_before_marker = false;
    while Instant::now() < deadline {
        if h.state_home.exists() {
            for entry in fs::read_dir(&h.state_home).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || !name.ends_with(".json") {
                    continue;
                }
                if !marker.exists() {
                    saw_pending_before_marker = true;
                }
            }
        }
        if saw_pending_before_marker {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let records = h.load_records();
    assert_eq!(records.len(), 1);
    let rec = &records[0];
    assert_eq!(rec["execution"]["outcome"], "completed");
    assert_eq!(rec["execution"]["started"], true);
    assert!(rec["execution"]["pid"].as_u64().is_some());
    assert!(
        saw_pending_before_marker || rec["execution"]["pid"].as_u64().is_some(),
        "expected pending-before-marker or finalized pid"
    );
    assert!(marker.exists(), "child should have touched marker");
}

#[test]
fn native_exit_codes_pass_through() {
    for code in [0u8, 1, 5, 6, 10, 126, 127, 255] {
        let mut h = Harness::new();
        h.install_python_child("demo-bin", "exit_with.py");
        h.set_demo_build("demo-bin", &[&code.to_string()], &["PATH"]);
        let out = h.run(&["--json", "build", "demo", "--execute"]);
        assert_eq!(
            out.status.code(),
            Some(code as i32),
            "code={code}: {}",
            stderr(&out)
        );
        let v = parse_json(&out);
        assert_eq!(v["exit_code"], code);
        assert_eq!(v["data"]["execution"]["exit_code"], code);
        assert_eq!(v["data"]["execution"]["outcome"], "completed");
    }
}

#[test]
fn json_one_document_with_bounded_child_capture() {
    let mut h = Harness::new();
    let py = python_executable();
    let script = format!(
        "#!{}\nimport sys\nsys.stdout.write('x' * {n})\n",
        py.display(),
        n = DEFAULT_LIMIT_BYTES + 64
    );
    fs::write(h.path_dir.join("demo-bin"), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            h.path_dir.join("demo-bin"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    h.set_demo_build("demo-bin", &[], &["PATH"]);

    let out = h.run(&["--json", "build", "demo", "--execute"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_one_json_document(stdout(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["mode"], "executed");
    assert_eq!(v["child"]["truncated"], true);
    let captured = v["child"]["stdout"].as_str().unwrap_or("").len();
    assert!(
        captured <= DEFAULT_LIMIT_BYTES,
        "captured {captured} exceeds limit {DEFAULT_LIMIT_BYTES}"
    );
}

// S6.1-05 regression: stderr overflow must not drop under-limit stdout (independent finalize).
#[test]
fn human_mode_rtk_eligible_asymmetric_overflow_still_emits_under_limit_stream() {
    let mut h = Harness::new();
    h.enable_rtk_compressor();

    let py = python_executable();
    let script = format!(
        "#!{}\nimport sys\nsys.stdout.write('SMALL_STDOUT_MARKER')\nsys.stdout.flush()\nsys.stderr.write('e' * {n})\nsys.stderr.flush()\n",
        py.display(),
        n = DEFAULT_LIMIT_BYTES + 1024,
    );
    fs::write(h.path_dir.join("demo-bin"), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            h.path_dir.join("demo-bin"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    h.set_demo_build("demo-bin", &[], &["PATH"]);

    let out = h.run(&["build", "demo", "--execute"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("SMALL_STDOUT_MARKER"),
        "the under-limit stdout stream must still be emitted once even when stderr overflows: stdout={:?}",
        stdout(&out)
    );
}

// S6.1-05 / §7.6: stdout over limit, stderr under limit — peer must still emit.
#[test]
fn human_mode_stdout_overflow_still_emits_under_limit_stderr() {
    let mut h = Harness::new();
    h.enable_rtk_compressor();

    let py = python_executable();
    let script = format!(
        "#!{}\nimport sys\nsys.stdout.write('o' * {n})\nsys.stdout.flush()\nsys.stderr.write('SMALL_STDERR_MARKER')\nsys.stderr.flush()\n",
        py.display(),
        n = DEFAULT_LIMIT_BYTES + 1024,
    );
    fs::write(h.path_dir.join("demo-bin"), script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            h.path_dir.join("demo-bin"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    h.set_demo_build("demo-bin", &[], &["PATH"]);

    let out = h.run(&["build", "demo", "--execute"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("SMALL_STDERR_MARKER"),
        "the under-limit stderr stream must still be emitted once even when stdout overflows: stderr={:?}",
        stderr(&out)
    );
}

#[test]
fn gate_and_deny_execute_never_spawn() {
    for (program, args, expect_exit, outcome) in [
        (
            "ontarch",
            &["bin-cleanup", "--mode", "dry-run"][..],
            POLICY_GATE,
            "gated",
        ),
        ("rm", &["bin/foo"][..], POLICY_DENY, "denied"),
    ] {
        let mut h = Harness::new();
        h.set_demo_build(program, args, &["PATH"]);
        let out = h.run(&["--json", "build", "demo", "--execute"]);
        assert_eq!(
            out.status.code(),
            Some(expect_exit as i32),
            "program={program}: {}",
            stderr(&out)
        );
        h.assert_marker_untouched();
        let rec = &h.load_records()[0];
        assert_eq!(rec["execution"]["outcome"], outcome);
        assert!(rec.get("resolution").is_none() || rec["resolution"].is_null());
        assert!(rec["execution"]["pid"].is_null());
    }
}

// S6.1-11: `collect_runtime_context`'s bounded invalid-opaque-id diagnostic must reach both the
// durable record's `error` field and the command envelope, not be silently discarded.
#[test]
fn invalid_tmux_pane_diagnostic_reaches_record_and_envelope() {
    let h = Harness::new();
    let out = h.run_env(
        &["--json", "build", "demo", "--execute"],
        &[("TMUX", "/tmp/tmux-1000/default"), ("TMUX_PANE", "%3/bad")],
    );
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    let diags = v["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.iter().any(|d| d["code"] == "runtime_context_invalid"),
        "envelope diagnostics must surface the runtime-context warning: {diags:?}"
    );
    let rec = &h.load_records()[0];
    assert_eq!(rec["execution"]["outcome"], "completed");
    assert!(rec["runtime_context"].is_null());
    assert_eq!(rec["error"]["code"], "runtime_context_invalid");
    assert!(!rec["error"]["message"].as_str().unwrap().contains('/'));
}

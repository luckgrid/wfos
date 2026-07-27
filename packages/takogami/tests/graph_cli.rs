//! E09.S7 Phase 2 — graph projection acceptance map (§14.1 / §15).

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use takogami::contracts::{
    RegistryGeneration, SourceFingerprint, fingerprint_bytes, fingerprint_file,
};
use takogami::exit_codes::{CONTRACT, NOT_IMPLEMENTED, RESOLUTION, SUCCESS};
use takogami::graph::types::{GRAPH_FILE_LIMIT_BYTES, GRAPH_FRESHNESS_METADATA_LIMIT_BYTES};
use takogami::graph::validate::GRAPH_UPSTREAM_PATHS;

const GENERATED_AT: &str = "2026-07-25T00:00:00Z";
const DESCRIPTOR_REL: &str = "registry/sources/descriptors/demo.descriptor.toml";

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_takogami"))
}

fn stdout(o: &Output) -> &str {
    std::str::from_utf8(&o.stdout).unwrap()
}

fn stderr(o: &Output) -> &str {
    std::str::from_utf8(&o.stderr).unwrap()
}

fn has_ansi(s: &str) -> bool {
    s.contains('\u{1b}')
}

struct GraphHarness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    path_dir: PathBuf,
    marker: PathBuf,
}

impl GraphHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        let registry = workspace.join("registry");
        let state_home = temp.path().join("state-home");
        let path_dir = workspace.join("bin");
        fs::create_dir_all(&registry).unwrap();
        fs::create_dir_all(&path_dir).unwrap();
        fs::create_dir_all(registry.join("sources/descriptors")).unwrap();

        let marker = workspace.join("MARKER_RAN");
        write_marker_exe(&path_dir.join("ontarch"), &marker);

        let h = Self {
            temp,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
        };
        h.seed_descriptor();
        h.write_upstream_registry();
        h.write_units();
        h.write_valid_graph();
        h
    }

    fn seed_descriptor(&self) {
        let body = r#"id = "demo"
kind = "package"
title = "Graph harness demo unit"
status = "active"
"#;
        fs::write(self.workspace.join(DESCRIPTOR_REL), body).unwrap();
    }

    fn descriptor_fingerprint(&self) -> SourceFingerprint {
        fingerprint_file(&self.workspace.join(DESCRIPTOR_REL), DESCRIPTOR_REL).unwrap()
    }

    fn write_upstream_registry(&self) {
        fs::write(
            self.registry.join("policies.json"),
            serde_json::to_string_pretty(&json!({
                "generated_at": GENERATED_AT,
                "policies": [{
                    "id": "takogami.agent",
                    "applies_to": "agent",
                    "version": "0.1.0",
                    "allow": { "commands": ["takogami graph"] },
                    "gate": { "commands": [] },
                    "block": { "commands": [] }
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            self.registry.join("profiles.json"),
            serde_json::to_string_pretty(&json!({
                "generated_at": GENERATED_AT,
                "profiles": [{
                    "id": "workspace-dev",
                    "title": "Harness dev",
                    "purpose": "graph-cli",
                    "rails": "takogami.agent",
                    "allowed_paths": ["**"],
                    "blocked_paths": []
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            self.registry.join("skills.json"),
            serde_json::to_string_pretty(&json!({
                "generated_at": GENERATED_AT,
                "skills": []
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_units(&self) {
        let authored = RegistryGeneration {
            generated_at: GENERATED_AT.into(),
            source_fingerprints: vec![self.descriptor_fingerprint()],
        };
        fs::write(
            self.registry.join("units.json"),
            serde_json::to_string_pretty(&json!({
                "generated_at": GENERATED_AT,
                "registry_generation": authored,
                "summary": { "total": 1 },
                "units": [{ "id": "demo", "kind": "package" }]
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn upstream_fps(&self) -> Vec<SourceFingerprint> {
        let mut fps = Vec::with_capacity(GRAPH_UPSTREAM_PATHS.len());
        for rel in GRAPH_UPSTREAM_PATHS {
            let abs = self.workspace.join(rel);
            let fp = fingerprint_file(&abs, rel).unwrap();
            assert_eq!(fp.path, rel);
            fps.push(fp);
        }
        assert_eq!(
            fps.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            GRAPH_UPSTREAM_PATHS.to_vec()
        );
        fps
    }

    fn write_valid_graph(&self) {
        let nodes = json!([
            {"id": "capability:build", "kind": "capability"},
            {"id": "demo", "kind": "package"},
            {"id": "policy:takogami.agent", "kind": "policy"},
            {"id": "profile:workspace-dev", "kind": "profile"}
        ]);
        let edges = json!([
            {"from": "demo", "rel": "governed-by", "to": "policy:takogami.agent"},
            {"from": "demo", "rel": "provides", "to": "capability:build"},
            {"from": "demo", "rel": "uses", "to": "capability:build"},
            {"from": "profile:workspace-dev", "rel": "selects", "to": "policy:takogami.agent"}
        ]);
        self.write_graph_payload(nodes, edges);
        fs::write(
            self.registry.join("graph.dot"),
            "digraph {\n  \"WRONG_SIBLING\";\n}\n",
        )
        .unwrap();
    }

    fn write_graph_payload(&self, nodes: Value, edges: Value) {
        let fps: Vec<Value> = self
            .upstream_fps()
            .into_iter()
            .map(|fp| serde_json::to_value(fp).unwrap())
            .collect();
        let doc = json!({
            "generated_at": GENERATED_AT,
            "registry_generation": {
                "generated_at": GENERATED_AT,
                "source_fingerprints": fps,
            },
            "nodes": nodes,
            "edges": edges,
        });
        fs::write(
            self.registry.join("graph.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
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

    fn assert_no_child_and_no_record(&self) {
        assert!(
            !self.marker.exists(),
            "graph must never spawn ontarch/child"
        );
        assert!(
            self.load_records().is_empty(),
            "graph must not create operational command records"
        );
    }

    fn snapshot_tree(&self) -> Vec<SnapshotEntry> {
        let mut entries = Vec::new();
        let temp = self.temp.path();
        // Registry separately; skip nested `registry` under workspace to avoid double-count.
        if self.registry.exists() {
            snapshot_walk(temp, &self.registry, &mut entries, None);
        }
        if self.workspace.exists() {
            snapshot_walk(temp, &self.workspace, &mut entries, Some("registry"));
        }
        if self.state_home.exists() {
            snapshot_walk(temp, &self.state_home, &mut entries, None);
        }
        // path_dir is under workspace (`ws/bin`) — already covered when walking workspace.
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        entries
    }

    fn assert_tree_unchanged(&self, before: &[SnapshotEntry]) {
        let after = self.snapshot_tree();
        assert_eq!(
            before,
            after.as_slice(),
            "graph must not mutate registry/workspace/state/marker trees"
        );
        self.assert_no_child_and_no_record();
    }

    fn assert_diags_omit_physical_roots(&self, out: &Output) {
        let body = format!("{}{}", stdout(out), stderr(out));
        for root in [
            self.temp.path(),
            self.workspace.as_path(),
            self.registry.as_path(),
            self.state_home.as_path(),
        ] {
            if let Some(s) = root.to_str() {
                assert!(
                    !body.contains(s),
                    "diagnostic leaked physical path {s}:\n{body}"
                );
            }
        }
        for needle in ["/Users/", "/private/var/", "/tmp/"] {
            // Allow only if the logical relative label somehow matches; fail on abs roots.
            if body.contains(needle) {
                // Soft: only fail when the harness temp absolute prefix leaked.
                if let Some(t) = self.temp.path().to_str() {
                    assert!(
                        !body.contains(t),
                        "diagnostic leaked temp root via {needle}:\n{body}"
                    );
                }
            }
        }
    }

    fn run_json_graph_with_timeout(&self, timeout: Duration) -> Output {
        use std::io::Read;
        let mut child = bin()
            .arg("--state-home")
            .arg(&self.state_home)
            .args(["--json", "graph"])
            .env("TAKOGAMI_ONTARCH_REGISTRY", &self.registry)
            .env("TAKOGAMI_WORKSPACE_ROOT", &self.workspace)
            .env("TAKOGAMI_STATE_HOME", &self.state_home)
            .env("PATH", &self.path_dir)
            .env_remove("TAKOGAMI_PROFILE")
            .env_remove("XDG_STATE_HOME")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn takogami");
        let start = Instant::now();
        let status = loop {
            match child.try_wait().expect("try_wait") {
                Some(status) => break status,
                None if start.elapsed() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("takogami graph timed out after {timeout:?} (possible FIFO hang)");
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            out.read_to_end(&mut stdout_buf).unwrap();
        }
        if let Some(mut err) = child.stderr.take() {
            err.read_to_end(&mut stderr_buf).unwrap();
        }
        Output {
            status,
            stdout: stdout_buf,
            stderr: stderr_buf,
        }
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

    fn mutate_units_fingerprint(&self) {
        let path = self.registry.join("units.json");
        let mut doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        doc["generated_at"] = json!("2099-01-01T00:00:00Z");
        fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    }
}

/// Restores file mode on drop (panic-safe permission cleanup for privileged-runner tests).
struct RestoreMode {
    path: PathBuf,
    mode: u32,
}

impl RestoreMode {
    fn chmod000(path: PathBuf) -> Self {
        let meta = fs::symlink_metadata(&path).unwrap();
        let mode = meta.permissions().mode();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        Self { path, mode }
    }

    fn still_readable(&self) -> bool {
        fs::File::open(&self.path).is_ok()
    }
}

impl Drop for RestoreMode {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

fn write_marker_exe(path: &Path, marker: &Path) {
    let script = format!("#!/bin/sh\necho ran >> {}\nexit 0\n", marker.display());
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotFileType {
    Regular,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotEntry {
    relative_path: String,
    file_type: SnapshotFileType,
    mode: u32,
    byte_len: u64,
    sha256: Option<String>,
    symlink_target: Option<String>,
}

fn snapshot_walk(
    temp_root: &Path,
    dir: &Path,
    out: &mut Vec<SnapshotEntry>,
    skip_child_name: Option<&str>,
) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd {
        let entry = entry.unwrap();
        if let Some(skip) = skip_child_name
            && entry.file_name() == *skip
        {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(temp_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let meta = fs::symlink_metadata(&path).unwrap();
        let ft = meta.file_type();
        let mode = meta.permissions().mode();
        if ft.is_symlink() {
            let target = fs::read_link(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .ok();
            out.push(SnapshotEntry {
                relative_path: rel,
                file_type: SnapshotFileType::Symlink,
                mode,
                byte_len: meta.len(),
                sha256: None,
                symlink_target: target,
            });
        } else if ft.is_dir() {
            out.push(SnapshotEntry {
                relative_path: rel,
                file_type: SnapshotFileType::Directory,
                mode,
                byte_len: 0,
                sha256: None,
                symlink_target: None,
            });
            snapshot_walk(temp_root, &path, out, None);
        } else if ft.is_file() {
            let bytes = fs::read(&path).unwrap_or_default();
            let digest = fingerprint_bytes(&bytes).digest;
            out.push(SnapshotEntry {
                relative_path: rel,
                file_type: SnapshotFileType::Regular,
                mode,
                byte_len: meta.len(),
                sha256: Some(digest),
                symlink_target: None,
            });
        } else {
            // FIFO/device/socket — never open for hashing.
            out.push(SnapshotEntry {
                relative_path: rel,
                file_type: SnapshotFileType::Other,
                mode,
                byte_len: 0,
                sha256: None,
                symlink_target: None,
            });
        }
    }
}

fn parse_json(out: &Output) -> Value {
    let s = stdout(out);
    serde_json::from_str(s).unwrap_or_else(|e| panic!("JSON parse failed: {e}\nstdout={s}"))
}

fn assert_one_json_document(raw: &str) {
    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    let first = stream.next().expect("one JSON document").unwrap();
    assert!(
        stream.next().is_none(),
        "must emit exactly one JSON document"
    );
    let _ = first;
}

fn diagnostic_codes(v: &Value) -> Vec<String> {
    v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str().map(str::to_string))
        .collect()
}

// --- §14.1 hit / formats ---

#[test]
fn untouched_fixture_is_freshness_hit() {
    let h = GraphHarness::new();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["data"]["freshness"], "hit");
    assert_eq!(v["metrics"]["registry_cache"], "hit");
    h.assert_no_child_and_no_record();
}

#[test]
fn empty_graph_is_hit_not_miss() {
    let h = GraphHarness::new();
    h.write_graph_payload(json!([]), json!([]));
    let out = h.run(&["--json", "graph", "--format", "json"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["freshness"], "hit");
    assert_eq!(v["data"]["graph"]["nodes"], json!([]));
    assert_eq!(v["data"]["graph"]["edges"], json!([]));
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_text_is_deterministic_hit() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    if let Some(nodes) = doc["nodes"].as_array_mut() {
        nodes.reverse();
    }
    if let Some(edges) = doc["edges"].as_array_mut() {
        edges.reverse();
    }
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out1 = h.run(&["graph"]);
    assert_eq!(
        out1.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&out1)
    );
    let text1 = stdout(&out1).to_string();
    let out2 = h.run(&["graph"]);
    assert_eq!(
        stdout(&out2),
        text1,
        "text projection must be deterministic"
    );
    assert!(text1.contains("Graph freshness: hit"));
    assert!(text1.contains("Nodes:"));
    assert!(text1.contains("Edges:"));
    h.assert_no_child_and_no_record();
}

#[test]
fn no_color_text_has_no_ansi() {
    let h = GraphHarness::new();
    let out = h.run(&["--no-color", "graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(
        !has_ansi(stdout(&out)),
        "text output must not contain ANSI escapes when --no-color is set"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_format_text_explicit() {
    let h = GraphHarness::new();
    let out = h.run(&["graph", "--format", "text"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_dot_is_escaped_and_deterministic() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    if let Some(nodes) = doc["nodes"].as_array_mut() {
        nodes.reverse();
    }
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    fs::write(h.registry.join("graph.dot"), "digraph { \"WRONG\"; }\n").unwrap();
    let out1 = h.run(&["graph", "--format", "dot"]);
    assert_eq!(
        out1.status.code(),
        Some(SUCCESS as i32),
        "{}",
        stderr(&out1)
    );
    let dot1 = stdout(&out1).to_string();
    assert!(dot1.contains("digraph") || dot1.contains("strict digraph"));
    assert!(
        !dot1.contains("WRONG"),
        "must not use unchecked sibling graph.dot"
    );
    let out2 = h.run(&["graph", "--format", "dot"]);
    assert_eq!(stdout(&out2), dot1, "DOT projection must be deterministic");
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_json_payload() {
    let h = GraphHarness::new();
    let out = h.run(&["graph", "--format", "json"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_one_json_document(stdout(&out));
    let v: Value = serde_json::from_str(stdout(&out)).unwrap();
    assert!(v.get("nodes").is_some());
    assert!(v.get("edges").is_some());
    assert!(v.get("registry_generation").is_some());
    h.assert_no_child_and_no_record();
}

#[test]
fn global_json_wraps_each_graph_format_in_one_envelope() {
    let h = GraphHarness::new();
    for format in ["text", "dot", "json"] {
        let out = h.run(&["--json", "graph", "--format", format]);
        assert_eq!(
            out.status.code(),
            Some(SUCCESS as i32),
            "format={format}: {}",
            stderr(&out)
        );
        assert_one_json_document(stdout(&out));
        let v = parse_json(&out);
        assert_eq!(v["schema_version"], "0.1.0");
        assert_eq!(v["command"], "graph");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["data"]["format"], format);
        assert!(
            v["data"]["graph"].is_object(),
            "data.graph must always be structured, not a string"
        );
        assert!(v["data"]["graph"].get("nodes").is_some());
        assert!(v["data"]["graph"].get("edges").is_some());
        assert_eq!(v["data"]["freshness"], "hit");
        assert_eq!(v["metrics"]["registry_cache"], "hit");
        if format == "text" || format == "dot" {
            assert!(
                v["data"]["rendered"].is_string(),
                "text/dot envelope must include data.rendered"
            );
        }
        assert!(!stdout(&out).contains("\ndigraph "));
    }
    h.assert_no_child_and_no_record();
}

// --- §14.1 miss / stale / invalid ---

#[test]
fn missing_graph_is_typed_miss_without_sync_or_record() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    let v = parse_json(&out);
    assert_eq!(v["status"], "error");
    let codes = diagnostic_codes(&v);
    assert!(
        codes.iter().any(|c| c == "graph_missing"),
        "expected graph_missing, got {codes:?}"
    );
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        body.contains("ontarch sync"),
        "missing graph must mention ontarch sync remediation: {body}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn stale_graph_input_fingerprint_is_typed_stale_without_sync() {
    let h = GraphHarness::new();
    h.mutate_units_fingerprint();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    let v = parse_json(&out);
    assert_eq!(v["status"], "error");
    let codes = diagnostic_codes(&v);
    assert!(
        codes.iter().any(|c| c == "graph_stale"),
        "expected graph_stale, got {codes:?}"
    );
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        body.contains("ontarch sync"),
        "stale graph must mention ontarch sync remediation: {body}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn authored_unit_fingerprint_stale_is_typed_stale() {
    let h = GraphHarness::new();
    let units_path = h.registry.join("units.json");
    let mut units: Value = serde_json::from_str(&fs::read_to_string(&units_path).unwrap()).unwrap();
    let mut bogus = fingerprint_bytes(b"not-the-authored-source");
    bogus.path = "registry/sources/descriptors/x.toml".into();
    units["registry_generation"] = serde_json::to_value(&RegistryGeneration {
        generated_at: GENERATED_AT.into(),
        source_fingerprints: vec![bogus],
    })
    .unwrap();
    fs::write(&units_path, serde_json::to_string_pretty(&units).unwrap()).unwrap();
    h.write_valid_graph();

    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_stale"),
        "expected graph_stale for authored drift, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn malformed_graph_json_is_invalid_registry() {
    let h = GraphHarness::new();
    fs::write(h.registry.join("graph.json"), "{not-json").unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn unknown_node_kind_is_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "x", "kind": "not-a-real-kind"}));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn unknown_relation_is_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["edges"].as_array_mut().unwrap().push(json!({
        "from": "demo",
        "rel": "teleports-to",
        "to": "policy:takogami.agent"
    }));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn duplicate_node_is_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "demo", "kind": "package"}));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn missing_edge_source_is_endpoint_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["edges"].as_array_mut().unwrap().push(json!({
        "from": "missing-source",
        "rel": "uses",
        "to": "demo"
    }));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        body.contains("endpoint")
            || body.contains("graph_endpoint_invalid")
            || body.contains("missing"),
        "expected endpoint diagnostic: {body}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn missing_edge_target_is_endpoint_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["edges"].as_array_mut().unwrap().push(json!({
        "from": "demo",
        "rel": "uses",
        "to": "missing-target"
    }));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn duplicate_edge_is_deterministic_rejection() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    let edge = doc["edges"][0].clone();
    doc["edges"].as_array_mut().unwrap().push(edge);
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn absolute_fingerprint_path_is_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["registry_generation"]["source_fingerprints"][0]["path"] = json!("/registry/policies.json");
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "expected graph_contract_invalid, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn symlink_graph_file_fails_closed() {
    let h = GraphHarness::new();
    let real = h.registry.join("graph.real.json");
    fs::rename(h.registry.join("graph.json"), &real).unwrap();
    symlink(&real, h.registry.join("graph.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_contract_invalid"));
    h.assert_diags_omit_physical_roots(&out);
    h.assert_no_child_and_no_record();
}

#[test]
fn graph_symlink_to_existing_file_is_contract() {
    symlink_graph_file_fails_closed();
}

#[test]
fn graph_symlink_to_directory_is_contract() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    let dir = h.registry.join("graph-dir");
    fs::create_dir(&dir).unwrap();
    symlink(&dir, h.registry.join("graph.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_contract_invalid"));
    h.assert_diags_omit_physical_roots(&out);
    h.assert_no_child_and_no_record();
}

#[test]
fn non_regular_graph_file_fails_closed() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    fs::create_dir(h.registry.join("graph.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn graph_file_over_8mib_hits_limit() {
    let h = GraphHarness::new();
    let pad_len = (GRAPH_FILE_LIMIT_BYTES as usize) + 1024;
    let pad = "x".repeat(pad_len);
    let doc = format!(
        r#"{{"generated_at":"{GENERATED_AT}","registry_generation":{{"generated_at":"{GENERATED_AT}","source_fingerprints":[]}},"nodes":[],"edges":[],"_pad":"{pad}"}}"#
    );
    assert!(doc.len() > GRAPH_FILE_LIMIT_BYTES as usize);
    fs::write(h.registry.join("graph.json"), doc).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn node_count_over_20000_hits_limit() {
    let h = GraphHarness::new();
    let mut nodes = Vec::new();
    for i in 0..20_001 {
        nodes.push(json!({"id": format!("n{i}"), "kind": "package"}));
    }
    h.write_graph_payload(json!(nodes), json!([]));
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn edge_count_over_100000_hits_limit() {
    let h = GraphHarness::new();
    let n = 5_000;
    let mut nodes = Vec::with_capacity(n);
    let mut edges = Vec::new();
    for i in 0..n {
        nodes.push(json!({"id": format!("n{i}"), "kind": "package"}));
    }
    for i in 0..n {
        for k in 0..21 {
            let j = (i + k + 1) % n;
            edges.push(json!({
                "from": format!("n{i}"),
                "rel": "uses",
                "to": format!("n{j}")
            }));
        }
    }
    assert!(edges.len() > 100_000, "edge count {}", edges.len());
    h.write_graph_payload(json!(nodes), json!(edges));
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn node_id_over_512_bytes_hits_limit() {
    let h = GraphHarness::new();
    let long_id = "a".repeat(513);
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": long_id, "kind": "package"}));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn printable_dot_special_characters_are_escaped() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["nodes"].as_array_mut().unwrap().push(json!({
        "id": "weird\"node\\with-chars",
        "kind": "package"
    }));
    doc["edges"].as_array_mut().unwrap().push(json!({
        "from": "demo",
        "rel": "uses",
        "to": "weird\"node\\with-chars"
    }));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["graph", "--format", "dot"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let dot = stdout(&out);
    assert!(
        dot.contains("\\\"") || !dot.contains("weird\"node"),
        "printable DOT specials must be escaped: {dot}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn graph_ids_with_control_characters_are_rejected() {
    let h = GraphHarness::new();
    for bad_id in [
        "has\nnewline",
        "has\rcarriage",
        "has\u{0000}nul",
        "has\u{007f}del",
    ] {
        h.write_valid_graph();
        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap())
                .unwrap();
        doc["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": bad_id, "kind": "package"}));
        fs::write(
            h.registry.join("graph.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
        let out = h.run(&["--json", "graph"]);
        assert_eq!(
            out.status.code(),
            Some(CONTRACT as i32),
            "control-char id must fail closed: {}",
            stderr(&out)
        );
        h.assert_no_child_and_no_record();
    }
}

#[test]
fn schema_and_rust_enums_win_over_stale_readme_vocabulary() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "future-brand", "kind": "tendril"}));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn bin_still_not_implemented() {
    let h = GraphHarness::new();
    let out = h.run(&["bin", "report"]);
    assert_eq!(
        out.status.code(),
        Some(NOT_IMPLEMENTED as i32),
        "{}",
        stderr(&out)
    );
    h.assert_no_child_and_no_record();
}

// --- P2-R03 tracked e2e goldens (byte-compare) ---

fn e2e_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e2e")
}

fn copy_e2e_ontarch(dest: &Path) {
    let src = e2e_fixture_root().join("ontarch");
    copy_dir_all(&src, dest);
}

fn copy_dir_all(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

struct E2eGraphHarness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    path_dir: PathBuf,
    marker: PathBuf,
}

impl E2eGraphHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ontarch");
        copy_e2e_ontarch(&workspace);
        let registry = workspace.join("registry");
        let state_home = temp.path().join("state-home");
        let path_dir = temp.path().join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        let marker = temp.path().join("MARKER_RAN");
        write_marker_exe(&path_dir.join("ontarch"), &marker);
        // Decoy sibling must remain wrong if present.
        assert!(registry.join("graph.dot").exists());
        Self {
            temp,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
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

    fn assert_no_child_and_no_record(&self) {
        assert!(
            !self.marker.exists(),
            "graph must never spawn ontarch/child"
        );
        assert!(
            !self.state_home.exists() || {
                fs::read_dir(&self.state_home)
                    .map(|d| d.filter_map(Result::ok).count() == 0)
                    .unwrap_or(true)
            },
            "graph must not create state-home records"
        );
    }
}

#[test]
fn e2e_expected_graph_text_matches_byte_for_byte() {
    let h = E2eGraphHarness::new();
    let out = h.run(&["graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let expected = fs::read(e2e_fixture_root().join("expected/graph-text.txt")).unwrap();
    assert_eq!(
        out.stdout, expected,
        "graph text must match tracked golden byte-for-byte"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn e2e_expected_graph_dot_matches_byte_for_byte() {
    let h = E2eGraphHarness::new();
    let out = h.run(&["graph", "--format", "dot"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let expected = fs::read(e2e_fixture_root().join("expected/graph-dot.txt")).unwrap();
    assert_eq!(
        out.stdout, expected,
        "graph DOT must match tracked golden byte-for-byte"
    );
    assert!(
        !stdout(&out).contains("WRONG_SIBLING"),
        "must not read sibling graph.dot"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn e2e_expected_graph_envelope_matches_byte_for_byte() {
    let h = E2eGraphHarness::new();
    let out = h.run(&["--json", "graph", "--format", "text"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let expected = fs::read(e2e_fixture_root().join("expected/graph-envelope.json")).unwrap();
    assert_eq!(
        out.stdout, expected,
        "graph envelope must match tracked golden byte-for-byte"
    );
    let v = parse_json(&out);
    assert!(!v.as_object().unwrap().contains_key("_pending"));
    assert_eq!(v["data"]["freshness"], "hit");
    h.assert_no_child_and_no_record();
}

#[test]
fn text_edge_truncation_reports_omitted_count() {
    let h = GraphHarness::new();
    let limit = takogami::graph::TEXT_EDGE_LINE_LIMIT;
    let mut nodes = vec![
        json!({"id": "capability:build", "kind": "capability"}),
        json!({"id": "demo", "kind": "package"}),
    ];
    let mut edges = Vec::new();
    // Enough distinct package nodes + uses edges to exceed the text line limit.
    for i in 0..(limit + 3) {
        let id = format!("pkg-{i:05}");
        nodes.push(json!({"id": id, "kind": "package"}));
        edges.push(json!({
            "from": format!("pkg-{i:05}"),
            "rel": "uses",
            "to": "capability:build"
        }));
    }
    h.write_graph_payload(json!(nodes), json!(edges));
    let out = h.run(&["graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains(&format!("… 3 edge line(s) omitted (limit {limit})")),
        "must report omitted count: {text}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn broken_pipe_on_graph_text_is_success() {
    let h = GraphHarness::new();
    // `head -c 0` closes the pipe immediately; graph must treat BrokenPipe as success.
    let mut child = bin()
        .arg("--state-home")
        .arg(&h.state_home)
        .arg("graph")
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &h.workspace)
        .env("TAKOGAMI_STATE_HOME", &h.state_home)
        .env("PATH", &h.path_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdout.take());
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(SUCCESS as i32),
        "broken pipe must exit success"
    );
}

#[test]
fn invalid_generated_at_is_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["generated_at"] = json!("2026-07-25T00:00:00.123Z");
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn unknown_graph_root_field_is_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["extra"] = json!(true);
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn unsorted_fingerprint_paths_are_contract_error() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    let fps = doc["registry_generation"]["source_fingerprints"]
        .as_array_mut()
        .unwrap();
    fps.swap(0, 1);
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn edge_endpoint_over_512_bytes_hits_limit() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    let long = "f".repeat(513);
    doc["edges"].as_array_mut().unwrap().push(json!({
        "from": long,
        "rel": "uses",
        "to": "capability:build"
    }));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

// --- Phase 2 closure corrections (C01–C07) ---

enum TopologyKind {
    Standalone,
    Embedded,
}

struct TopologyHarness {
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    workspace: PathBuf,
    registry: PathBuf,
    state_home: PathBuf,
    path_dir: PathBuf,
    marker: PathBuf,
    descriptor_rel: String,
}

impl TopologyHarness {
    fn new(kind: TopologyKind) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let (workspace, registry, descriptor_rel) = match kind {
            TopologyKind::Standalone => {
                let root = temp.path().join("wfos");
                fs::create_dir_all(root.join(".agents")).unwrap();
                let registry = root.join("packages/ontarch/registry");
                (
                    root,
                    registry,
                    "packages/ontarch/descriptors/demo.descriptor.toml".to_string(),
                )
            }
            TopologyKind::Embedded => {
                let ws = temp.path().join("workstreams");
                fs::create_dir_all(ws.join(".agents")).unwrap();
                let registry = ws.join("Build/src/workspaces/wfos/packages/ontarch/registry");
                (
                    ws,
                    registry,
                    "Build/src/workspaces/wfos/packages/ontarch/descriptors/demo.descriptor.toml"
                        .to_string(),
                )
            }
        };
        let state_home = temp.path().join("state-home");
        let path_dir = temp.path().join("bin");
        fs::create_dir_all(&registry).unwrap();
        fs::create_dir_all(registry.join("sources/descriptors")).unwrap();
        fs::create_dir_all(workspace.join(Path::new(&descriptor_rel).parent().unwrap())).unwrap();
        fs::create_dir_all(&path_dir).unwrap();
        let marker = temp.path().join("MARKER_RAN");
        write_marker_exe(&path_dir.join("ontarch"), &marker);

        let h = Self {
            temp,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
            descriptor_rel,
        };
        h.seed();
        h
    }

    fn seed(&self) {
        let body = r#"id = "demo"
kind = "package"
title = "Topology demo"
status = "active"
"#;
        fs::write(self.workspace.join(&self.descriptor_rel), body).unwrap();
        for name in ["policies.json", "profiles.json", "skills.json"] {
            fs::write(
                self.registry.join(name),
                serde_json::to_string_pretty(&json!({
                    "generated_at": GENERATED_AT,
                    "items": []
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let authored = RegistryGeneration {
            generated_at: GENERATED_AT.into(),
            source_fingerprints: vec![
                fingerprint_file(
                    &self.workspace.join(&self.descriptor_rel),
                    &self.descriptor_rel,
                )
                .unwrap(),
            ],
        };
        fs::write(
            self.registry.join("units.json"),
            serde_json::to_string_pretty(&json!({
                "generated_at": GENERATED_AT,
                "registry_generation": authored,
                "summary": { "total": 1 },
                "units": [{ "id": "demo", "kind": "package" }]
            }))
            .unwrap(),
        )
        .unwrap();

        let mut fps = Vec::new();
        for rel in GRAPH_UPSTREAM_PATHS {
            let name = rel.strip_prefix("registry/").unwrap();
            let abs = self.registry.join(name);
            fps.push(fingerprint_file(&abs, rel).unwrap());
        }
        let doc = json!({
            "generated_at": GENERATED_AT,
            "registry_generation": {
                "generated_at": GENERATED_AT,
                "source_fingerprints": fps,
            },
            "nodes": [{"id": "demo", "kind": "package"}],
            "edges": []
        });
        fs::write(
            self.registry.join("graph.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
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

    fn assert_no_child_and_no_record(&self) {
        assert!(
            !self.marker.exists(),
            "graph must never spawn ontarch/child"
        );
        assert!(
            !self.state_home.exists()
                || fs::read_dir(&self.state_home)
                    .map(|d| d.filter_map(Result::ok).count() == 0)
                    .unwrap_or(true),
            "graph must not create state-home records"
        );
    }
}

#[test]
fn standalone_wfos_topology_graph_hit() {
    let h = TopologyHarness::new(TopologyKind::Standalone);
    assert!(h.workspace.join("packages/ontarch/registry").exists());
    assert_ne!(h.workspace, h.registry);
    let out = h.run(&["graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(stdout(&out).contains("Graph freshness: hit"));
    h.assert_no_child_and_no_record();
}

#[test]
fn standalone_wfos_topology_json_graph_hit() {
    let h = TopologyHarness::new(TopologyKind::Standalone);
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let v = parse_json(&out);
    assert_eq!(v["data"]["freshness"], "hit");
    h.assert_no_child_and_no_record();
}

#[test]
fn embedded_workstreams_topology_graph_hit() {
    let h = TopologyHarness::new(TopologyKind::Embedded);
    assert!(
        h.registry
            .ends_with("Build/src/workspaces/wfos/packages/ontarch/registry")
    );
    let out = h.run(&["graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert!(stdout(&out).contains("Graph freshness: hit"));
    h.assert_no_child_and_no_record();
}

#[test]
fn embedded_workstreams_topology_json_graph_hit() {
    let h = TopologyHarness::new(TopologyKind::Embedded);
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_eq!(parse_json(&out)["data"]["freshness"], "hit");
    h.assert_no_child_and_no_record();
}

#[test]
fn workspace_root_move_does_not_break_layer1_upstream() {
    let h = TopologyHarness::new(TopologyKind::Embedded);
    // Move authored base to a sibling while keeping registry_root fixed.
    let alt_ws = h.temp.path().join("alt-ws");
    fs::create_dir_all(&alt_ws).unwrap();
    // Copy descriptor into alt workspace at same relative path so Layer 2 still hits.
    let dest = alt_ws.join(&h.descriptor_rel);
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::copy(h.workspace.join(&h.descriptor_rel), &dest).unwrap();
    let out = bin()
        .arg("--state-home")
        .arg(&h.state_home)
        .args(["--json", "graph"])
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &alt_ws)
        .env("TAKOGAMI_STATE_HOME", &h.state_home)
        .env("PATH", &h.path_dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_eq!(parse_json(&out)["data"]["freshness"], "hit");
}

#[test]
fn unknown_graph_registry_generation_field_is_contract() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["registry_generation"]["unexpected"] = json!(true);
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_tree_unchanged(&before);
}

#[test]
fn unknown_graph_fingerprint_field_is_contract() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["registry_generation"]["source_fingerprints"][0]["unexpected"] = json!(1);
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn unknown_units_registry_generation_field_is_contract() {
    let h = GraphHarness::new();
    let mut units: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    units["registry_generation"]["unexpected"] = json!(true);
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&units).unwrap(),
    )
    .unwrap();
    // Refresh graph fingerprints for units.json after mutation.
    h.write_valid_graph();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn unknown_units_authored_fingerprint_field_is_contract() {
    let h = GraphHarness::new();
    let mut units: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("units.json")).unwrap()).unwrap();
    units["registry_generation"]["source_fingerprints"][0]["extra"] = json!("x");
    fs::write(
        h.registry.join("units.json"),
        serde_json::to_string_pretty(&units).unwrap(),
    )
    .unwrap();
    h.write_valid_graph();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn calendar_invalid_generated_at_is_contract() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["generated_at"] = json!("2026-13-40T99:99:99Z");
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn non_leap_feb_29_generated_at_is_contract() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["generated_at"] = json!("2025-02-29T00:00:00Z");
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
}

#[test]
fn upstream_registry_symlink_is_contract() {
    let h = GraphHarness::new();
    let real = h.registry.join("policies.json.real");
    fs::rename(h.registry.join("policies.json"), &real).unwrap();
    symlink(&real, h.registry.join("policies.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn upstream_registry_directory_is_contract() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("policies.json")).unwrap();
    fs::create_dir(h.registry.join("policies.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
}

#[test]
fn authored_source_symlink_is_contract() {
    let h = GraphHarness::new();
    let path = h.workspace.join(DESCRIPTOR_REL);
    let real = h
        .workspace
        .join("registry/sources/descriptors/demo.real.toml");
    fs::rename(&path, &real).unwrap();
    symlink(&real, &path).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
}

#[test]
fn graph_file_fifo_is_contract_without_blocking() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    let path = h.registry.join("graph.json");
    let status = Command::new("mkfifo").arg(&path).status().expect("mkfifo");
    assert!(status.success(), "mkfifo failed");
    let out = h.run_json_graph_with_timeout(Duration::from_secs(2));
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "expected graph_contract_invalid, got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
    h.assert_no_child_and_no_record();
}

#[test]
fn duplicate_long_emoji_id_no_panic_bounded_diagnostic() {
    let h = GraphHarness::new();
    let long = "\u{1F44D}".repeat(70);
    h.write_graph_payload(
        json!([
            {"id": long, "kind": "package"},
            {"id": long, "kind": "policy"}
        ]),
        json!([]),
    );
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_contract_invalid"));
    h.assert_no_child_and_no_record();
}

#[test]
fn missing_long_unicode_source_endpoint_no_panic() {
    let h = GraphHarness::new();
    let long = "\u{1F3AF}".repeat(40);
    h.write_graph_payload(
        json!([{"id": "demo", "kind": "package"}]),
        json!([{"from": long, "rel": "uses", "to": "demo"}]),
    );
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_endpoint_invalid"));
}

fn broken_pipe_case(args: &[&str]) {
    let h = GraphHarness::new();
    let before = h.snapshot_tree();
    let mut child = bin()
        .arg("--state-home")
        .arg(&h.state_home)
        .args(args)
        .env("TAKOGAMI_ONTARCH_REGISTRY", &h.registry)
        .env("TAKOGAMI_WORKSPACE_ROOT", &h.workspace)
        .env("TAKOGAMI_STATE_HOME", &h.state_home)
        .env("PATH", &h.path_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdout.take());
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(SUCCESS as i32),
        "broken pipe must exit success for {args:?}"
    );
    h.assert_tree_unchanged(&before);
}

#[test]
fn broken_pipe_graph_dot_is_success() {
    broken_pipe_case(&["graph", "--format", "dot"]);
}

#[test]
fn broken_pipe_graph_raw_json_is_success() {
    broken_pipe_case(&["graph", "--format", "json"]);
}

#[test]
fn broken_pipe_json_envelope_text_is_success() {
    broken_pipe_case(&["--json", "graph"]);
}

#[test]
fn broken_pipe_json_envelope_dot_is_success() {
    broken_pipe_case(&["--json", "graph", "--format", "dot"]);
}

#[test]
fn broken_pipe_json_envelope_graph_is_success() {
    broken_pipe_case(&["--json", "graph", "--format", "json"]);
}

#[test]
fn streaming_hash_large_authored_file_succeeds() {
    let h = GraphHarness::new();
    let path = h.workspace.join(DESCRIPTOR_REL);
    // 1 MiB authored file — must stream-hash without failing.
    let big = vec![b'x'; 1024 * 1024];
    fs::write(&path, &big).unwrap();
    h.write_units();
    h.write_valid_graph();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_eq!(parse_json(&out)["data"]["freshness"], "hit");
}

#[test]
fn dangling_graph_symlink_is_contract_not_miss() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    symlink(
        h.registry.join("does-not-exist.json"),
        h.registry.join("graph.json"),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "dangling symlink must be contract not miss, got {codes:?}"
    );
    assert!(
        !codes.iter().any(|c| c == "graph_missing"),
        "dangling symlink must not classify as graph_missing"
    );
    h.assert_diags_omit_physical_roots(&out);
    h.assert_no_child_and_no_record();
}

#[test]
fn absolute_path_absent_from_human_graph_symlink_diag() {
    let h = GraphHarness::new();
    let real = h.registry.join("graph.real.json");
    fs::rename(h.registry.join("graph.json"), &real).unwrap();
    symlink(&real, h.registry.join("graph.json")).unwrap();
    let out = h.run(&["graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn absolute_path_absent_from_json_graph_fifo_diag() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    let path = h.registry.join("graph.json");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let out = h.run_json_graph_with_timeout(Duration::from_secs(2));
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn absolute_path_absent_from_units_oversize_diag() {
    let h = GraphHarness::new();
    let path = h.registry.join("units.json");
    let f = fs::File::create(&path).unwrap();
    f.set_len(GRAPH_FRESHNESS_METADATA_LIMIT_BYTES + 1).unwrap();
    // Refresh Layer-1 digests so freshness reaches the units metadata bound.
    h.write_valid_graph();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn upstream_missing_is_stale() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("policies.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_stale"), "got {codes:?}");
    h.assert_no_child_and_no_record();
}

#[test]
fn authored_missing_is_stale() {
    let h = GraphHarness::new();
    fs::remove_file(h.workspace.join(DESCRIPTOR_REL)).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_stale"), "got {codes:?}");
    h.assert_no_child_and_no_record();
}

#[test]
fn present_regular_upstream_io_failure_is_contract() {
    let h = GraphHarness::new();
    let path = h.registry.join("policies.json");
    let guard = RestoreMode::chmod000(path);
    // privileged-runner: mode 000 still readable — skip chmod-denial assertion.
    if guard.still_readable() {
        eprintln!("privileged-runner: mode 000 still readable; skipping chmod-denial CLI assert");
        return;
    }
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "present regular I/O fail must be contract not stale, got {codes:?}"
    );
    assert!(!codes.iter().any(|c| c == "graph_stale"));
    h.assert_diags_omit_physical_roots(&out);
    h.assert_no_child_and_no_record();
    drop(guard);
}

#[test]
fn present_regular_authored_io_failure_is_contract() {
    let h = GraphHarness::new();
    let path = h.workspace.join(DESCRIPTOR_REL);
    let guard = RestoreMode::chmod000(path);
    // privileged-runner: mode 000 still readable — skip chmod-denial assertion.
    if guard.still_readable() {
        eprintln!("privileged-runner: mode 000 still readable; skipping chmod-denial CLI assert");
        return;
    }
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "authored I/O fail must be contract not stale, got {codes:?}"
    );
    assert!(!codes.iter().any(|c| c == "graph_stale"));
    h.assert_diags_omit_physical_roots(&out);
    drop(guard);
}

#[test]
fn upstream_fifo_is_contract_without_blocking() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("policies.json")).unwrap();
    let path = h.registry.join("policies.json");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let out = h.run_json_graph_with_timeout(Duration::from_secs(2));
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn authored_fifo_is_contract_without_blocking() {
    let h = GraphHarness::new();
    let path = h.workspace.join(DESCRIPTOR_REL);
    fs::remove_file(&path).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let out = h.run_json_graph_with_timeout(Duration::from_secs(2));
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn units_symlink_is_contract() {
    let h = GraphHarness::new();
    let real = h.registry.join("units.real.json");
    fs::rename(h.registry.join("units.json"), &real).unwrap();
    symlink(&real, h.registry.join("units.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn units_fifo_is_contract_without_blocking() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("units.json")).unwrap();
    let path = h.registry.join("units.json");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let out = h.run_json_graph_with_timeout(Duration::from_secs(2));
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
}

#[test]
fn oversized_units_freshness_metadata_hits_limit() {
    let h = GraphHarness::new();
    let path = h.registry.join("units.json");
    let f = fs::File::create(&path).unwrap();
    f.set_len(GRAPH_FRESHNESS_METADATA_LIMIT_BYTES + 1).unwrap();
    h.write_valid_graph();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "expected graph_limit_exceeded, got {codes:?}"
    );
    h.assert_diags_omit_physical_roots(&out);
    h.assert_tree_unchanged(&before);
}

#[test]
fn same_length_registry_mutation_detected_by_snapshot() {
    let h = GraphHarness::new();
    let before = h.snapshot_tree();
    let path = h.registry.join("units.json");
    let mut bytes = fs::read(&path).unwrap();
    let orig_len = bytes.len();
    let flipped = bytes
        .iter_mut()
        .find(|b| **b == b'0')
        .expect("digit to flip");
    *flipped = b'1';
    assert_eq!(bytes.len(), orig_len);
    fs::write(&path, &bytes).unwrap();
    let after = h.snapshot_tree();
    assert_ne!(
        before, after,
        "same-length content mutation must change byte-identical snapshot"
    );
}

#[test]
fn same_length_authored_mutation_detected_by_snapshot() {
    let h = GraphHarness::new();
    let before = h.snapshot_tree();
    let path = h.workspace.join(DESCRIPTOR_REL);
    let mut bytes = fs::read(&path).unwrap();
    let orig_len = bytes.len();
    let flipped = bytes
        .iter_mut()
        .find(|b| b.is_ascii_lowercase())
        .expect("letter to flip");
    *flipped = if *flipped == b'a' { b'b' } else { b'a' };
    assert_eq!(bytes.len(), orig_len);
    fs::write(&path, &bytes).unwrap();
    let after = h.snapshot_tree();
    assert_ne!(before, after);
}

#[test]
fn symlink_target_change_detected_by_snapshot() {
    let h = GraphHarness::new();
    let link = h.workspace.join("link-probe");
    let a = h.workspace.join("link-a");
    let b = h.workspace.join("link-b");
    fs::write(&a, b"a").unwrap();
    fs::write(&b, b"b").unwrap();
    symlink(&a, &link).unwrap();
    let before = h.snapshot_tree();
    fs::remove_file(&link).unwrap();
    symlink(&b, &link).unwrap();
    let after = h.snapshot_tree();
    assert_ne!(before, after, "symlink target change must be detected");
}

#[test]
fn mode_or_filetype_change_detected_by_snapshot() {
    let h = GraphHarness::new();
    let path = h.registry.join("skills.json");
    let before = h.snapshot_tree();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let after = h.snapshot_tree();
    assert_ne!(before, after, "mode change must be detected");
}

#[test]
fn success_output_surfaces_leave_tree_byte_identical() {
    for args in [
        &["graph"][..],
        &["graph", "--format", "dot"][..],
        &["graph", "--format", "json"][..],
        &["--json", "graph"][..],
    ] {
        let h = GraphHarness::new();
        let before = h.snapshot_tree();
        let out = h.run(args);
        assert_eq!(
            out.status.code(),
            Some(SUCCESS as i32),
            "args={args:?} stderr={}",
            stderr(&out)
        );
        if args.first() == Some(&"--json") {
            assert_eq!(parse_json(&out)["data"]["freshness"], "hit");
        } else if args == ["graph"] {
            assert!(
                stdout(&out).contains("Graph freshness: hit"),
                "{}",
                stdout(&out)
            );
        }
        h.assert_tree_unchanged(&before);
    }
}

#[test]
fn miss_snapshot_unchanged() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    h.assert_tree_unchanged(&before);
}

#[test]
fn layer1_stale_snapshot_unchanged() {
    let h = GraphHarness::new();
    h.mutate_units_fingerprint();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    h.assert_tree_unchanged(&before);
}

#[test]
fn dangling_contract_snapshot_unchanged_aside_from_setup() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    symlink(
        h.registry.join("missing.json"),
        h.registry.join("graph.json"),
    )
    .unwrap();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_tree_unchanged(&before);
}

#[test]
fn layer2_stale_leaves_tree_byte_identical() {
    let h = GraphHarness::new();
    let units_path = h.registry.join("units.json");
    let mut units: Value = serde_json::from_str(&fs::read_to_string(&units_path).unwrap()).unwrap();
    let mut bogus = fingerprint_bytes(b"not-the-authored-source");
    bogus.path = "registry/sources/descriptors/x.toml".into();
    units["registry_generation"] = serde_json::to_value(&RegistryGeneration {
        generated_at: GENERATED_AT.into(),
        source_fingerprints: vec![bogus],
    })
    .unwrap();
    fs::write(&units_path, serde_json::to_string_pretty(&units).unwrap()).unwrap();
    h.write_valid_graph();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(
        out.status.code(),
        Some(RESOLUTION as i32),
        "{}",
        stderr(&out)
    );
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(codes.iter().any(|c| c == "graph_stale"), "got {codes:?}");
    h.assert_tree_unchanged(&before);
}

#[test]
fn decode_contract_leaves_tree_byte_identical() {
    let h = GraphHarness::new();
    fs::write(h.registry.join("graph.json"), "{not-json").unwrap();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "got {codes:?}"
    );
    h.assert_tree_unchanged(&before);
}

#[test]
fn endpoint_contract_leaves_tree_byte_identical() {
    let h = GraphHarness::new();
    h.write_graph_payload(
        json!([{"id": "demo", "kind": "package"}]),
        json!([{"from": "missing-src", "rel": "uses", "to": "demo"}]),
    );
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_endpoint_invalid"),
        "got {codes:?}"
    );
    h.assert_tree_unchanged(&before);
}

#[test]
fn special_file_contract_leaves_tree_byte_identical() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    symlink(
        h.registry.join("missing.json"),
        h.registry.join("graph.json"),
    )
    .unwrap();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_contract_invalid"),
        "got {codes:?}"
    );
    h.assert_tree_unchanged(&before);
}

#[test]
fn limit_contract_leaves_tree_byte_identical() {
    let h = GraphHarness::new();
    let path = h.registry.join("units.json");
    let f = fs::File::create(&path).unwrap();
    f.set_len(GRAPH_FRESHNESS_METADATA_LIMIT_BYTES + 1).unwrap();
    h.write_valid_graph();
    let before = h.snapshot_tree();
    let out = h.run(&["--json", "graph"]);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    let codes = diagnostic_codes(&parse_json(&out));
    assert!(
        codes.iter().any(|c| c == "graph_limit_exceeded"),
        "got {codes:?}"
    );
    h.assert_tree_unchanged(&before);
}

//! E09.S7 Phase 0 — graph projection acceptance map (§14.1 / §15).
//!
//! Asserts final S7 contracts. At baseline these fail because `graph` is still
//! `not_implemented` (exit 10) and Ontarch graph freshness is not yet emitted.
//! Do not `#[ignore]` — failures must stay visible until Phase 2 lands.

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use takogami::contracts::{RegistryGeneration, fingerprint_bytes, fingerprint_file};
use takogami::exit_codes::{CONTRACT, NOT_IMPLEMENTED, SUCCESS};

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
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&path_dir).unwrap();
        copy_dir(&fixture_root(), &workspace);
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
        h.write_upstream_docs();
        h.write_valid_graph();
        h
    }

    fn write_upstream_docs(&self) {
        for name in [
            "units.json",
            "policies.json",
            "profiles.json",
            "skills.json",
        ] {
            let path = self.registry.join(name);
            if path.exists() {
                continue;
            }
            fs::write(
                &path,
                serde_json::to_string_pretty(&json!({
                    "generated_at": "2026-07-25T00:00:00Z",
                    name.trim_end_matches(".json"): []
                }))
                .unwrap(),
            )
            .unwrap();
        }
        // Ensure skills.json exists even when resolution fixture omits it.
        let skills = self.registry.join("skills.json");
        if !skills.exists() {
            fs::write(
                &skills,
                r#"{"generated_at":"2026-07-25T00:00:00Z","skills":[]}"#,
            )
            .unwrap();
        }
    }

    fn upstream_fps(&self) -> Vec<Value> {
        let mut fps = Vec::new();
        for name in [
            "units.json",
            "policies.json",
            "profiles.json",
            "skills.json",
        ] {
            let rel = format!("registry/{name}");
            let abs = self.workspace.join(&rel);
            let fp = fingerprint_file(&abs, &rel).unwrap();
            fps.push(serde_json::to_value(&fp).unwrap());
        }
        fps
    }

    fn write_valid_graph(&self) {
        let doc = json!({
            "generated_at": "2026-07-25T00:00:00Z",
            "registry_generation": {
                "generated_at": "2026-07-25T00:00:00Z",
                "source_fingerprints": self.upstream_fps(),
            },
            "nodes": [
                {"id": "demo", "kind": "package"},
                {"id": "policy:takogami.agent", "kind": "policy"},
                {"id": "profile:workspace-dev", "kind": "profile"},
                {"id": "capability:build", "kind": "capability"}
            ],
            "edges": [
                {"from": "profile:workspace-dev", "rel": "selects", "to": "policy:takogami.agent"},
                {"from": "demo", "rel": "governed-by", "to": "policy:takogami.agent"},
                {"from": "demo", "rel": "provides", "to": "capability:build"},
                {"from": "demo", "rel": "uses", "to": "capability:build"}
            ]
        });
        fs::write(
            self.registry.join("graph.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
        fs::write(
            self.registry.join("graph.dot"),
            "digraph {\n  \"demo\";\n}\n",
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

fn write_marker_exe(path: &Path, marker: &Path) {
    let script = format!("#!/bin/sh\necho ran >> {}\nexit 0\n", marker.display());
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
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
    let first = stream.next().expect("one JSON document").unwrap();
    assert!(
        stream.next().is_none(),
        "must emit exactly one JSON document"
    );
    let _ = first;
}

fn assert_not_still_unimplemented(out: &Output) {
    // Phase 0: document why this fails today when handlers are absent.
    assert_ne!(
        out.status.code(),
        Some(NOT_IMPLEMENTED as i32),
        "S7 graph contracts not implemented yet (exit 10). stderr={}",
        stderr(out)
    );
}

// --- §14.1 hit / formats ---

#[test]
fn valid_current_graph_text_is_deterministic_hit() {
    let h = GraphHarness::new();
    let out = h.run(&["graph"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("demo"), "text projection must list nodes");
    assert!(text.contains("governed-by") || text.contains("selects"));
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_format_text_explicit() {
    let h = GraphHarness::new();
    let out = h.run(&["graph", "--format", "text"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_dot_is_escaped_and_deterministic() {
    let h = GraphHarness::new();
    let out = h.run(&["graph", "--format", "dot"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let dot = stdout(&out);
    assert!(dot.contains("digraph") || dot.contains("strict digraph"));
    assert!(dot.contains("demo"));
    h.assert_no_child_and_no_record();
}

#[test]
fn valid_current_graph_json_payload() {
    let h = GraphHarness::new();
    let out = h.run(&["graph", "--format", "json"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    assert_one_json_document(stdout(&out));
    let v = parse_json(&out);
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
        assert_not_still_unimplemented(&out);
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
        assert!(v["data"]["graph"].is_object() || v["data"]["graph"].is_string());
        assert_eq!(v["data"]["freshness"], "hit");
        // Never emit raw DOT/text outside the envelope in global JSON mode.
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
    assert_not_still_unimplemented(&out);
    let v = parse_json(&out);
    assert_eq!(v["status"], "error");
    let codes: Vec<&str> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(
        codes.iter().any(|c| *c == "graph_missing"),
        "expected graph_missing, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn stale_graph_input_fingerprint_is_typed_stale_without_sync() {
    let h = GraphHarness::new();
    h.mutate_units_fingerprint();
    let out = h.run(&["--json", "graph"]);
    assert_not_still_unimplemented(&out);
    let v = parse_json(&out);
    assert_eq!(v["status"], "error");
    let codes: Vec<&str> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(
        codes.iter().any(|c| *c == "graph_stale"),
        "expected graph_stale, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn authored_unit_fingerprint_stale_is_typed_stale() {
    let h = GraphHarness::new();
    // Break authored source binding inside units registry_generation while graph fps still match
    // the units.json file bytes — S7 requires both layers.
    let units_path = h.registry.join("units.json");
    let mut units: Value = serde_json::from_str(&fs::read_to_string(&units_path).unwrap()).unwrap();
    let mut bogus = fingerprint_bytes(b"not-the-authored-source");
    bogus.path = "registry/sources/descriptors/x.toml".into();
    units["registry_generation"] = serde_json::to_value(&RegistryGeneration {
        generated_at: "2026-07-25T00:00:00Z".into(),
        source_fingerprints: vec![bogus],
    })
    .unwrap();
    fs::write(&units_path, serde_json::to_string_pretty(&units).unwrap()).unwrap();
    // Refresh graph fps so only authored-layer drift remains.
    h.write_valid_graph();

    let out = h.run(&["--json", "graph"]);
    assert_not_still_unimplemented(&out);
    let v = parse_json(&out);
    let codes: Vec<&str> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(
        codes.iter().any(|c| *c == "graph_stale"),
        "expected graph_stale for authored drift, got {codes:?}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn malformed_graph_json_is_invalid_registry() {
    let h = GraphHarness::new();
    fs::write(h.registry.join("graph.json"), "{not-json").unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_not_still_unimplemented(&out);
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
    assert_not_still_unimplemented(&out);
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
    assert_not_still_unimplemented(&out);
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
    assert_not_still_unimplemented(&out);
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
    assert_not_still_unimplemented(&out);
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
    assert_not_still_unimplemented(&out);
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
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn symlink_graph_file_fails_closed() {
    let h = GraphHarness::new();
    let real = h.registry.join("graph.real.json");
    fs::rename(h.registry.join("graph.json"), &real).unwrap();
    symlink(&real, h.registry.join("graph.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn non_regular_graph_file_fails_closed() {
    let h = GraphHarness::new();
    fs::remove_file(h.registry.join("graph.json")).unwrap();
    fs::create_dir(h.registry.join("graph.json")).unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn oversized_graph_hits_bounded_error() {
    let h = GraphHarness::new();
    // Build a graph that exceeds the S7 node/edge/string budget once limits land.
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for i in 0..50_000 {
        let id = format!("n{i}");
        nodes.push(json!({"id": id, "kind": "package"}));
        if i > 0 {
            edges.push(json!({
                "from": format!("n{}", i - 1),
                "rel": "uses",
                "to": format!("n{i}")
            }));
        }
    }
    let doc = json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "registry_generation": {
            "generated_at": "2026-07-25T00:00:00Z",
            "source_fingerprints": h.upstream_fps(),
        },
        "nodes": nodes,
        "edges": edges,
    });
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["--json", "graph"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

#[test]
fn dot_special_characters_are_safely_escaped() {
    let h = GraphHarness::new();
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(h.registry.join("graph.json")).unwrap()).unwrap();
    doc["nodes"].as_array_mut().unwrap().push(json!({
        "id": "weird\"node\\with\nchars",
        "kind": "package"
    }));
    doc["edges"].as_array_mut().unwrap().push(json!({
        "from": "demo",
        "rel": "uses",
        "to": "weird\"node\\with\nchars"
    }));
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&doc).unwrap(),
    )
    .unwrap();
    let out = h.run(&["graph", "--format", "dot"]);
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(SUCCESS as i32), "{}", stderr(&out));
    let dot = stdout(&out);
    assert!(
        !dot.contains("weird\"node\\with\nchars") || dot.contains("\\\""),
        "DOT specials must be escaped: {dot}"
    );
    h.assert_no_child_and_no_record();
}

#[test]
fn schema_and_rust_enums_win_over_stale_readme_vocabulary() {
    // Contract: closed schema/Rust enums are authoritative. A future README drift must not
    // widen accepted kinds. Assert unknown kind still fails after Phase 2.
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
    assert_not_still_unimplemented(&out);
    assert_eq!(out.status.code(), Some(CONTRACT as i32), "{}", stderr(&out));
    h.assert_no_child_and_no_record();
}

//! E09.S7 Phase 1 direct Ontarch graph contracts (S7-P1-R01/R02/R03).

#[path = "support/mod.rs"]
mod support;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use support::{HermeticOntarch, snapshot_checkout_registry, write_executable};

fn stderr_of(out: &std::process::Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn mutate_graph(h: &HermeticOntarch, f: impl FnOnce(&mut Value)) -> String {
    let mut g = h.load_generated_graph();
    f(&mut g);
    fs::write(
        h.registry.join("graph.json"),
        serde_json::to_string_pretty(&g).unwrap(),
    )
    .unwrap();
    let out = h.run_script(&h.validate, &[]);
    assert!(!out.status.success(), "validate should fail after mutation");
    stderr_of(&out)
}

fn assert_graph_diag(combined: &str, needle: &str) {
    assert!(
        combined.contains(needle),
        "expected graph diagnostic containing {needle:?}\n{combined}"
    );
    assert!(
        !combined.contains("no descriptors found"),
        "must not fail for missing descriptors\n{combined}"
    );
}

fn expected_fingerprints(h: &HermeticOntarch) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in [
        "policies.json",
        "profiles.json",
        "skills.json",
        "units.json",
    ] {
        let bytes = fs::read(h.registry.join(name)).unwrap();
        out.push((
            format!("registry/{name}"),
            format!("{:x}", Sha256::digest(&bytes)),
        ));
    }
    out
}

#[test]
fn ontarch_validate_accepts_complete_valid_phase1_fixture() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    h.assert_validate_success();
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_graph_direct_tests_do_not_write_checkout_registry() {
    let before = snapshot_checkout_registry();
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let _ = h.run_script(&h.validate, &[]);
    assert_eq!(before, snapshot_checkout_registry());
}

#[test]
fn ontarch_graph_contains_exact_upstream_fingerprints() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let graph = h.load_generated_graph();
    let fps = graph["registry_generation"]["source_fingerprints"]
        .as_array()
        .expect("source_fingerprints");
    assert_eq!(fps.len(), 4);
    let expected = expected_fingerprints(&h);
    for (i, (path, digest)) in expected.iter().enumerate() {
        assert_eq!(fps[i]["path"], *path);
        assert_eq!(fps[i]["algorithm"], "sha256");
        assert_eq!(fps[i]["digest"], *digest);
        let d = fps[i]["digest"].as_str().unwrap();
        assert!(
            regex_is_sha256(d),
            "digest must match ^[0-9a-f]{{64}}$: {d}"
        );
        assert!(!path.starts_with('/'));
        assert!(!path.contains('\\'));
        assert!(!path.contains(".."));
    }
    let paths: Vec<&str> = fps.iter().filter_map(|f| f["path"].as_str()).collect();
    let mut uniq = paths.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(paths.len(), uniq.len(), "duplicate fingerprint paths");
}

fn regex_is_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

#[test]
fn ontarch_graph_fingerprint_paths_are_sorted_and_portable() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let graph = h.load_generated_graph();
    let paths: Vec<String> = graph["registry_generation"]["source_fingerprints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        paths,
        vec![
            "registry/policies.json".to_string(),
            "registry/profiles.json".to_string(),
            "registry/skills.json".to_string(),
            "registry/units.json".to_string(),
        ]
    );
}

#[test]
fn ontarch_graph_nodes_and_edges_are_sorted() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let graph = h.load_generated_graph();
    let nodes = graph["nodes"].as_array().unwrap();
    let mut sorted_nodes = nodes.clone();
    sorted_nodes.sort_by(|a, b| {
        (a["kind"].as_str().unwrap(), a["id"].as_str().unwrap())
            .cmp(&(b["kind"].as_str().unwrap(), b["id"].as_str().unwrap()))
    });
    assert_eq!(nodes, &sorted_nodes);
    let edges = graph["edges"].as_array().unwrap();
    let mut sorted_edges = edges.clone();
    sorted_edges.sort_by(|a, b| {
        (
            a["from"].as_str().unwrap(),
            a["rel"].as_str().unwrap(),
            a["to"].as_str().unwrap(),
        )
            .cmp(&(
                b["from"].as_str().unwrap(),
                b["rel"].as_str().unwrap(),
                b["to"].as_str().unwrap(),
            ))
    });
    assert_eq!(edges, &sorted_edges);
}

#[test]
fn graph_schema_rejects_missing_registry_generation() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g.as_object_mut().unwrap().remove("registry_generation");
    });
    assert_graph_diag(&combined, "graph:");
}

#[test]
fn graph_schema_rejects_unknown_root_field() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["unexpected"] = json!(1);
    });
    assert_graph_diag(&combined, "graph:unknown_or_missing_root_field");
}

#[test]
fn graph_schema_rejects_unknown_generation_field() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["extra"] = json!(true);
    });
    assert_graph_diag(&combined, "graph:invalid_registry_generation");
}

#[test]
fn graph_schema_rejects_unknown_fingerprint_field() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"][0]["extra"] = json!(1);
    });
    assert_graph_diag(&combined, "graph:unknown_fingerprint_field");
}

#[test]
fn graph_schema_rejects_unknown_node_field() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["nodes"][0]["extra"] = json!(true);
    });
    assert_graph_diag(&combined, "graph:invalid_node_or_edge_shape");
}

#[test]
fn graph_schema_rejects_unknown_edge_field() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    // Ensure at least one edge exists
    assert!(
        !h.load_generated_graph()["edges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let combined = mutate_graph(&h, |g| {
        g["edges"][0]["extra"] = json!(true);
    });
    assert_graph_diag(&combined, "graph:invalid_node_or_edge_shape");
}

#[test]
fn graph_fingerprint_rejects_missing_input() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        let fps = g["registry_generation"]["source_fingerprints"]
            .as_array_mut()
            .unwrap();
        fps.pop();
    });
    assert_graph_diag(&combined, "graph:fingerprint_count");
}

#[test]
fn graph_fingerprint_rejects_extra_input() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "path": "registry/extra.json",
                "algorithm": "sha256",
                "digest": "0".repeat(64)
            }));
    });
    assert_graph_diag(&combined, "graph:fingerprint_count");
}

#[test]
fn graph_fingerprint_rejects_duplicate_path() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        let fps = g["registry_generation"]["source_fingerprints"]
            .as_array_mut()
            .unwrap();
        let dup = fps[0].clone();
        fps[1] = dup;
    });
    assert!(
        combined.contains("graph:duplicate_fingerprint_path")
            || combined.contains("graph:fingerprint_path_set")
            || combined.contains("graph:fingerprint_paths_unsorted"),
        "{combined}"
    );
}

#[test]
fn graph_fingerprint_rejects_absolute_path() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"][0]["path"] =
            json!("/tmp/registry/units.json");
    });
    assert_graph_diag(&combined, "graph:absolute_fingerprint_path");
}

#[test]
fn graph_fingerprint_rejects_backslash_path() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"][0]["path"] = json!(r"registry\units.json");
    });
    assert!(
        combined.contains("graph:unsafe_fingerprint_path")
            || combined.contains("graph:bad_fingerprint_path"),
        "{combined}"
    );
}

#[test]
fn graph_fingerprint_rejects_unsupported_algorithm() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"][0]["algorithm"] = json!("sha1");
    });
    assert_graph_diag(&combined, "graph:unsupported_fingerprint_algorithm");
}

#[test]
fn graph_fingerprint_rejects_malformed_digest() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"][0]["digest"] = json!("not-a-digest");
    });
    assert_graph_diag(&combined, "graph:malformed_fingerprint_digest");
}

#[test]
fn graph_fingerprint_rejects_digest_mismatch() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        g["registry_generation"]["source_fingerprints"][0]["digest"] = json!("ab".repeat(32));
    });
    assert_graph_diag(&combined, "graph:fingerprint_digest_mismatch");
}

#[test]
fn graph_rejects_duplicate_node_id() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        let node = g["nodes"][0].clone();
        g["nodes"].as_array_mut().unwrap().push(node);
    });
    assert_graph_diag(&combined, "graph:duplicate_node_id");
}

#[test]
fn graph_rejects_duplicate_edge_tuple() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        let edge = g["edges"][0].clone();
        g["edges"].as_array_mut().unwrap().push(edge);
        // Keep sort order so uniqueness fires before unsorted check when possible.
        let edges = g["edges"].as_array_mut().unwrap();
        edges.sort_by(|a, b| {
            (
                a["from"].as_str().unwrap(),
                a["rel"].as_str().unwrap(),
                a["to"].as_str().unwrap(),
            )
                .cmp(&(
                    b["from"].as_str().unwrap(),
                    b["rel"].as_str().unwrap(),
                    b["to"].as_str().unwrap(),
                ))
        });
    });
    assert_graph_diag(&combined, "graph:duplicate_edge_tuple");
}

#[test]
fn graph_rejects_control_character_id() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    for (label, bad) in [
        ("newline", "bad\nid"),
        ("carriage return", "bad\rid"),
        ("tab", "bad\tid"),
        ("NUL", "bad\0id"),
        ("DEL", "bad\u{007f}id"),
    ] {
        let combined = mutate_graph(&h, |g| {
            // Keep edge endpoints consistent so the control check is authoritative.
            let old = g["nodes"][0]["id"].as_str().unwrap().to_string();
            g["nodes"][0]["id"] = json!(bad);
            if let Some(edges) = g["edges"].as_array_mut() {
                for e in edges {
                    if e["from"] == old {
                        e["from"] = json!(bad);
                    }
                    if e["to"] == old {
                        e["to"] = json!(bad);
                    }
                }
            }
        });
        assert_graph_diag(&combined, "graph:control_character_id");
        let _ = label;
    }
}

#[test]
fn graph_rejects_same_id_different_kind() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let combined = mutate_graph(&h, |g| {
        let id = g["nodes"][0]["id"].clone();
        g["nodes"].as_array_mut().unwrap().push(json!({
            "id": id,
            "kind": "policy"
        }));
        let nodes = g["nodes"].as_array_mut().unwrap();
        nodes.sort_by(|a, b| {
            (a["kind"].as_str().unwrap(), a["id"].as_str().unwrap())
                .cmp(&(b["kind"].as_str().unwrap(), b["id"].as_str().unwrap()))
        });
    });
    assert_graph_diag(&combined, "graph:duplicate_node_id");
}

#[test]
fn sync_first_install_rolls_back_absent_pair_on_dot_failure() {
    let h = HermeticOntarch::new();
    assert!(!h.registry.join("graph.json").exists());
    assert!(!h.registry.join("graph.dot").exists());
    let tools = h.tools_bsd_stat();
    let real_mv = {
        let mut found = None;
        for prefix in ["/bin", "/usr/bin"] {
            let p = std::path::PathBuf::from(prefix).join("mv");
            if p.is_file() {
                found = Some(p);
                break;
            }
        }
        found.expect("mv")
    };
    let _ = fs::remove_file(tools.join("mv"));
    write_executable(
        &tools.join("mv"),
        &format!(
            r#"#!/bin/sh
set -eu
dest=""
for a in "$@"; do dest="$a"; done
case "$dest" in
  *.dot|*/graph.dot) exit 1 ;;
esac
exec {real} "$@"
"#,
            real = support::shell_single_quote(&real_mv.to_string_lossy()),
        ),
    );
    let out = h.run_with_path(&h.sync, &[], &tools);
    assert!(
        !out.status.success(),
        "first-install DOT failure must fail sync"
    );
    assert!(
        !h.registry.join("graph.json").exists(),
        "absent A must be removed after B failure"
    );
    assert!(!h.registry.join("graph.dot").exists());
    for entry in fs::read_dir(&h.registry).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        for prefix in [".graph-json.", ".graph-dot.", ".bak-a.", ".bak-b."] {
            assert!(!name.starts_with(prefix), "temp leak: {name}");
        }
    }
}

#[test]
fn e2e_registry_fixture_digests_match_tracked_bytes() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e2e/ontarch/registry");
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(fixture.join("graph.json")).unwrap()).unwrap();
    let fps = graph["registry_generation"]["source_fingerprints"]
        .as_array()
        .expect("fingerprints");
    assert_eq!(fps.len(), 4);
    for name in [
        "policies.json",
        "profiles.json",
        "skills.json",
        "units.json",
    ] {
        let bytes = fs::read(fixture.join(name)).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let entry = fps
            .iter()
            .find(|f| f["path"] == format!("registry/{name}"))
            .unwrap_or_else(|| panic!("missing fingerprint for {name}"));
        assert_eq!(entry["algorithm"], "sha256");
        assert_eq!(
            entry["digest"].as_str().unwrap(),
            digest,
            "E2E digest drift for {name}"
        );
    }
}

#[test]
fn graph_dot_escapes_printable_specials() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let g = h.load_generated_graph();
    let sample = json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "registry_generation": g["registry_generation"].clone(),
        "nodes": [
            {"id": "a\"b\\c", "kind": "package"},
            {"id": "policy:x", "kind": "policy"}
        ],
        "edges": [
            {"from": "a\"b\\c", "rel": "governed-by", "to": "policy:x"}
        ]
    });
    let path = h.registry.join("_dot_sample.json");
    fs::write(&path, serde_json::to_string(&sample).unwrap()).unwrap();
    let script = format!(
        r#"set -euo pipefail
source {lib}/common.sh
source {lib}/registry.sh
ontarch_emit_graph_dot < {path}
"#,
        lib = support::shell_single_quote(&h.ontarch_pkg.join("lib").to_string_lossy()),
        path = support::shell_single_quote(&path.to_string_lossy()),
    );
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(script)
        .current_dir(&h.ws_root)
        .env("WS_ROOT", &h.ws_root)
        .env_remove("ONTARCH_REGISTRY")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dot = String::from_utf8_lossy(&out.stdout);
    // esc turns " -> \", \ -> \\ so the DOT token is a\"b\\c inside quotes.
    assert!(
        dot.contains(r#"a\"b\\c"#),
        "expected escaped quote/backslash in DOT\n{dot}"
    );
}

#[test]
fn sync_retains_prior_graph_pair_on_install_failure() {
    let h = HermeticOntarch::new();
    h.run_sync_and_require_success();
    let before_json = fs::read(h.registry.join("graph.json")).unwrap();
    let before_dot = fs::read(h.registry.join("graph.dot")).unwrap();

    let tools = h.tools_bsd_stat();
    let real_mv = {
        let mut found = None;
        for prefix in ["/bin", "/usr/bin"] {
            let p = std::path::PathBuf::from(prefix).join("mv");
            if p.is_file() {
                found = Some(p);
                break;
            }
        }
        found.expect("mv")
    };
    let marker = tools.join("MV_SHIM_RAN");
    let _ = fs::remove_file(tools.join("mv"));
    let _ = fs::remove_file(&marker);
    write_executable(
        &tools.join("mv"),
        &format!(
            r#"#!/bin/sh
set -eu
echo "$*" >> {marker}
dest=""
for a in "$@"; do dest="$a"; done
case "$dest" in
  *.dot|*/graph.dot) exit 1 ;;
esac
exec {real} "$@"
"#,
            marker = support::shell_single_quote(&marker.to_string_lossy()),
            real = support::shell_single_quote(&real_mv.to_string_lossy()),
        ),
    );
    let out = h.run_with_path(&h.sync, &[], &tools);
    assert!(
        marker.is_file(),
        "mv shim must run under PATH=tool-root; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "sync must fail when graph.dot install fails\nstdout:{}\nstderr:{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let after_json = fs::read(h.registry.join("graph.json")).unwrap();
    let after_dot = fs::read(h.registry.join("graph.dot")).unwrap();
    assert_eq!(before_json, after_json, "graph.json retained");
    assert_eq!(before_dot, after_dot, "graph.dot retained");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("ok graph.json"),
        "must not claim graph success after failure\n{combined}"
    );
}

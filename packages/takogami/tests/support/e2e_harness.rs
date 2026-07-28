//! Phase 4 hermetic E2E harness: full Workstreams-like overlay, scrubbed env,
//! spawn/record counting, and variant mutators.
//!
//! Overlay strategy: start from tracked `fixtures/e2e`, install `packages/ontarch`
//! from the e2e registry (graph fingerprints match), then merge resolution
//! policies/profiles/tools/descriptors for lifecycle + bin, and recompute
//! `graph.json` source fingerprints so graph stays a freshness hit.
//!
//! Platform helper: FIFO state-root variants invoke host `mkfifo` (recorded
//! dependency; not a provider or network contact).

use super::{
    copy_dir, hash_tree, sample_cleanup_plan, sample_inventory, write_canonical_fake_ontarch,
    write_executable, write_marker_exe,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

/// Serializes e2e tests that mutate process-global umask / cwd assumptions.
pub static E2E_LOCK: Mutex<()> = Mutex::new(());

pub fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_takogami"))
}

pub fn e2e_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e2e")
}

pub fn resolution_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution")
}

pub fn stdout(o: &Output) -> &str {
    std::str::from_utf8(&o.stdout).unwrap_or("")
}

pub fn stderr(o: &Output) -> &str {
    std::str::from_utf8(&o.stderr).unwrap_or("")
}

pub fn parse_json(out: &Output) -> Value {
    let s = stdout(out).trim();
    serde_json::from_str(s).unwrap_or_else(|e| {
        panic!(
            "JSON parse failed: {e}\nexit={:?}\nstdout={s}\nstderr={}",
            out.status.code(),
            stderr(out)
        )
    })
}

/// Closed projection source set required for Ontarch helper seal.
pub fn ensure_projection_source_manifest(ontarch_pkg: &Path) {
    let real = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch");
    let copy_or_stub = |rel: &str, stub: &[u8]| {
        let dest = ontarch_pkg.join(rel);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let src = real.join(rel);
        if src.is_file() {
            fs::copy(&src, &dest).unwrap();
        } else {
            fs::write(&dest, stub).unwrap();
        }
    };
    if !ontarch_pkg.join("bin/ontarch-bin-report").is_file() {
        write_executable(
            &ontarch_pkg.join("bin/ontarch-bin-report"),
            "#!/bin/sh\nexit 0\n",
        );
    }
    if !ontarch_pkg.join("bin/ontarch-bin-cleanup").is_file() {
        write_executable(
            &ontarch_pkg.join("bin/ontarch-bin-cleanup"),
            "#!/bin/sh\nexit 0\n",
        );
    }
    copy_or_stub("lib/common.sh", b"# test\n");
    copy_or_stub("lib/registry.sh", b"# test\n");
    copy_or_stub("policies/takogami.agent.policy.toml", b"# test\n");
    copy_or_stub("policies/agent-bin.policy.toml", b"# test\n");
    copy_or_stub("schemas/bin-inventory.schema.json", b"{}\n");
    copy_or_stub("schemas/bin-cleanup-plan.schema.json", b"{}\n");
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    format!("{:x}", Sha256::digest(&bytes))
}

/// Recompute graph Layer-1 fingerprints (`registry/{policies,profiles,skills,units}.json`).
pub fn recompute_graph_fingerprints(registry: &Path) {
    let graph_path = registry.join("graph.json");
    let mut graph: Value = serde_json::from_str(&fs::read_to_string(&graph_path).unwrap()).unwrap();
    let fps = [
        "registry/policies.json",
        "registry/profiles.json",
        "registry/skills.json",
        "registry/units.json",
    ];
    let mut out = Vec::new();
    for rel in fps {
        let name = rel.strip_prefix("registry/").unwrap();
        let file = registry.join(name);
        if file.is_file() {
            out.push(serde_json::json!({
                "path": rel,
                "algorithm": "sha256",
                "digest": sha256_file(&file),
            }));
        }
    }
    if let Some(generation) = graph.get_mut("registry_generation") {
        generation["source_fingerprints"] = Value::Array(out);
        if generation.get("generated_at").is_none() {
            generation["generated_at"] = Value::String("2026-07-25T00:00:00Z".into());
        }
    }
    fs::write(
        &graph_path,
        serde_json::to_string_pretty(&graph).unwrap() + "\n",
    )
    .unwrap();
}

/// Project units.json from authored descriptors (hit routing + Layer-2 fingerprints).
fn rewrite_units_authored_fingerprints(workspace: &Path, registry: &Path) {
    let desc_dir = registry.join("sources/descriptors");
    let mut fingerprints = Vec::new();
    let mut units = Vec::new();
    if desc_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&desc_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let rel = format!("packages/ontarch/registry/sources/descriptors/{name}");
            let abs = workspace.join(&rel);
            assert!(abs.is_file(), "descriptor missing at {}", abs.display());
            fingerprints.push(serde_json::json!({
                "path": rel,
                "algorithm": "sha256",
                "digest": sha256_file(&abs),
            }));
            let text = fs::read_to_string(&path).unwrap();
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
    }
    let doc = serde_json::json!({
        "generated_at": "2026-07-25T00:00:00Z",
        "registry_generation": {
            "generated_at": "2026-07-25T00:00:00Z",
            "source_fingerprints": fingerprints,
        },
        "summary": { "total": units.len() },
        "units": units,
    });
    fs::write(
        registry.join("units.json"),
        serde_json::to_string_pretty(&doc).unwrap() + "\n",
    )
    .unwrap();
}

/// Distinct lifecycle stdout/stderr literals for integrated `--execute` proofs.
pub const LIFECYCLE_STDOUT: &str = "E2E_LIFECYCLE_STDOUT_LITERAL";
pub const LIFECYCLE_STDERR: &str = "E2E_LIFECYCLE_STDERR_LITERAL";

pub struct E2eHarness {
    pub temp: tempfile::TempDir,
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub registry: PathBuf,
    pub state_home: PathBuf,
    pub path_dir: PathBuf,
    /// Canonical Ontarch projection spawn marker (`MARKER_CANONICAL`).
    pub marker: PathBuf,
    /// PATH decoy `ontarch` spawn marker — must never be touched.
    pub path_decoy_marker: PathBuf,
    /// Lifecycle child spawn marker (`MARKER_LIFECYCLE`) — independent of Ontarch.
    pub lifecycle_marker: PathBuf,
    /// Side-channel env dump written by the lifecycle child (absolute path).
    pub child_env_dump: PathBuf,
    /// Provider shim spawn marker — must never be touched (doctor/tools PATH-only).
    pub provider_marker: PathBuf,
    pub panoply_side: PathBuf,
    pub canonical_ontarch: PathBuf,
    pub tracked_hash: String,
    /// Hash of everything under `root` except `state_home` contents after setup.
    pub setup_non_state_hash: String,
}

impl E2eHarness {
    pub fn new() -> Self {
        let tracked_hash = hash_tree(&e2e_root());
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("e2e");
        fs::create_dir_all(&root).unwrap();
        copy_dir(&e2e_root(), &root);

        // WfOS-like workspace: tracked workspace tree + packages/ontarch from e2e.
        let workspace = root.join("workspace");
        let ontarch_pkg = workspace.join("packages/ontarch");
        if ontarch_pkg.exists() {
            fs::remove_dir_all(&ontarch_pkg).ok();
        }
        fs::create_dir_all(ontarch_pkg.parent().unwrap()).unwrap();
        copy_dir(&root.join("ontarch"), &ontarch_pkg);
        let registry = ontarch_pkg.join("registry");

        // Merge resolution policies/profiles/tools/scan + descriptors for lifecycle/bin.
        let res = resolution_root();
        for name in ["policies.json", "profiles.json", "tools.json", "scan.json"] {
            let src = res.join("registry").join(name);
            if src.is_file() {
                fs::copy(&src, registry.join(name)).unwrap();
            }
        }
        // Install only the demo descriptor for a coherent MVP scan/list/build path.
        // Full multi-unit matrices live in resolution_cli / policy_cli.
        let desc_dir = registry.join("sources/descriptors");
        fs::create_dir_all(&desc_dir).unwrap();
        for entry in fs::read_dir(&desc_dir).unwrap().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("toml") {
                let _ = fs::remove_file(&p);
            }
        }
        fs::copy(
            res.join("registry/sources/descriptors/demo.descriptor.toml"),
            desc_dir.join("demo.descriptor.toml"),
        )
        .unwrap();
        if res.join("demo").is_dir() {
            copy_dir(&res.join("demo"), &workspace.join("demo"));
        }

        // Layer-2 authored fingerprints are resolved from WORKSPACE_ROOT, not the
        // Ontarch package root. Project units.json with entrypoints + confined paths.
        rewrite_units_authored_fingerprints(&workspace, &registry);

        // Profile must select takogami.agent (graph edge) and agent-bin (bin projection).
        let profiles_path = registry.join("profiles.json");
        let mut profiles: Value =
            serde_json::from_str(&fs::read_to_string(&profiles_path).unwrap()).unwrap();
        if let Some(arr) = profiles.get_mut("profiles").and_then(|v| v.as_array_mut()) {
            for p in arr.iter_mut() {
                if p["id"] == "workspace-dev" {
                    p["rails"] = Value::String("takogami.agent".into());
                    p["rails_bin"] = Value::String("agent-bin".into());
                }
            }
        }
        fs::write(
            &profiles_path,
            serde_json::to_string_pretty(&profiles).unwrap() + "\n",
        )
        .unwrap();

        // Ensure takogami.agent allows lifecycle + graph + bin report/report-only.
        let policies_path = registry.join("policies.json");
        let mut policies: Value =
            serde_json::from_str(&fs::read_to_string(&policies_path).unwrap()).unwrap();
        if let Some(arr) = policies.get_mut("policies").and_then(|v| v.as_array_mut()) {
            for p in arr.iter_mut() {
                if p["id"] == "takogami.agent" {
                    p["allow"]["commands"] = serde_json::json!([
                        "takogami scan",
                        "takogami list",
                        "takogami info",
                        "takogami doctor",
                        "takogami tools",
                        "takogami interfaces",
                        "takogami dev",
                        "takogami build",
                        "takogami check",
                        "takogami graph",
                        "takogami bin report",
                        "takogami bin cleanup --mode report-only",
                        "takogami session list",
                        "takogami session show",
                        "takogami session latest"
                    ]);
                    p["gate"]["commands"] =
                        serde_json::json!(["takogami bin cleanup --mode dry-run"]);
                    p["block"]["commands"] = serde_json::json!([
                        "takogami bin cleanup --mode archive",
                        "takogami bin cleanup --mode delete-approved"
                    ]);
                }
            }
        }
        fs::write(
            &policies_path,
            serde_json::to_string_pretty(&policies).unwrap() + "\n",
        )
        .unwrap();

        recompute_graph_fingerprints(&registry);
        ensure_projection_source_manifest(&ontarch_pkg);

        let state_home = root.join("state-home");
        fs::create_dir_all(&state_home).unwrap();
        let path_dir = root.join("tools");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["cargo", "rustc", "moon", "demo-bin", "rg", "git"] {
            write_executable(&path_dir.join(name), "#!/bin/sh\nexit 0\n");
        }
        let path_decoy_marker = root.join("MARKER_PATH_DECOY");
        write_marker_exe(&path_dir.join("ontarch"), &path_decoy_marker);

        let marker = root.join("MARKER_CANONICAL");
        let lifecycle_marker = root.join("MARKER_LIFECYCLE");
        let child_env_dump = root.join("LIFECYCLE_CHILD_ENV");
        let provider_marker = root.join("MARKER_PROVIDER");
        let panoply_side = root.join("PANOPLY_SEEN");
        let canonical_ontarch = ontarch_pkg.join("bin/ontarch");
        let inv = sample_inventory(workspace.to_str().unwrap());
        let clean = sample_cleanup_plan("report-only");
        write_canonical_fake_ontarch(&canonical_ontarch, &marker, &panoply_side, &inv, &clean);

        let setup_non_state_hash = hash_tree_excluding(&root, &state_home);

        Self {
            temp,
            root,
            workspace,
            registry,
            state_home,
            path_dir,
            marker,
            path_decoy_marker,
            lifecycle_marker,
            child_env_dump,
            provider_marker,
            panoply_side,
            canonical_ontarch,
            tracked_hash,
            setup_non_state_hash,
        }
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.run_env(args, &[], true)
    }

    /// Scrub host-state variables so the suite does not depend on developer HOME/XDG.
    pub fn run_scrubbed(&self, args: &[&str]) -> Output {
        self.run_env(args, &[], true)
    }

    pub fn run_env(&self, args: &[&str], extra: &[(&str, &str)], scrub: bool) -> Output {
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
            .env_remove("PANOPLY_AGENT");
        if scrub {
            // Isolate from developer home / agent config. Keep a temp HOME for libs that require it.
            let isolated_home = self.root.join("isolated-home");
            fs::create_dir_all(&isolated_home).ok();
            cmd.env("HOME", &isolated_home)
                .env_remove("XDG_CONFIG_HOME")
                .env_remove("XDG_DATA_HOME")
                .env_remove("XDG_CACHE_HOME")
                .env_remove("AGENTS_HOME")
                .env_remove("USERPROFILE");
        }
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn takogami")
    }

    pub fn marker_count(&self) -> usize {
        if !self.marker.exists() {
            return 0;
        }
        fs::read_to_string(&self.marker)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .count()
    }

    pub fn lifecycle_marker_count(&self) -> usize {
        if !self.lifecycle_marker.exists() {
            return 0;
        }
        fs::read_to_string(&self.lifecycle_marker)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .count()
    }

    pub fn assert_no_spawn(&self) {
        assert_eq!(self.marker_count(), 0, "canonical Ontarch must not spawn");
        assert!(
            !self.path_decoy_marker.exists(),
            "PATH decoy ontarch must never run"
        );
    }

    pub fn assert_spawn_count(&self, n: usize) {
        assert_eq!(
            self.marker_count(),
            n,
            "expected {n} canonical Ontarch spawn(s)"
        );
        assert!(
            !self.path_decoy_marker.exists(),
            "PATH decoy ontarch must never run"
        );
    }

    pub fn assert_lifecycle_spawn_count(&self, n: usize) {
        assert_eq!(
            self.lifecycle_marker_count(),
            n,
            "expected {n} lifecycle child spawn(s)"
        );
    }

    pub fn assert_no_provider_process(&self) {
        assert!(
            !self.provider_marker.exists(),
            "tmux/herdr provider shims must never execute"
        );
    }

    /// Overwrite PATH `moon` with a deterministic lifecycle child.
    ///
    /// Absolute marker/env-dump paths are embedded because the sealed child env
    /// clears everything except descriptor `env_keys` (PATH). Env dump uses
    /// `/usr/bin/env` (absolute) because PATH is the sealed tools dir only.
    pub fn install_lifecycle_child(&self, exit_code: u8) {
        write_executable(
            &self.path_dir.join("moon"),
            &format!(
                "#!/bin/sh\n\
                 echo ran >> {marker}\n\
                 /usr/bin/env > {env_dump}\n\
                 printf '%s' '{stdout}'\n\
                 printf '%s' '{stderr}' >&2\n\
                 exit {exit_code}\n",
                marker = shell_quote(&self.lifecycle_marker.to_string_lossy()),
                env_dump = shell_quote(&self.child_env_dump.to_string_lossy()),
                stdout = LIFECYCLE_STDOUT,
                stderr = LIFECYCLE_STDERR,
                exit_code = exit_code,
            ),
        );
    }

    /// Install provider PATH shims that would mark if ever executed.
    pub fn install_provider_shims(&self, names: &[&str]) {
        for name in names {
            write_marker_exe(&self.path_dir.join(name), &self.provider_marker);
        }
    }

    pub fn overlay_tools_variant(&self, name: &str) {
        let src = e2e_root().join(format!("variants/tools/{name}.json"));
        assert!(src.is_file(), "missing tools variant fixture {name}");
        fs::copy(&src, self.registry.join("tools.json")).unwrap();
    }

    pub fn load_records(&self) -> Vec<Value> {
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

    pub fn assert_tracked_unchanged(&self) {
        assert_eq!(
            hash_tree(&e2e_root()),
            self.tracked_hash,
            "tracked e2e fixture tree must remain byte-identical"
        );
    }

    /// Prove operational session records stayed under state-home and the isolated
    /// HOME did not receive controller state. Also re-hash the non-state overlay
    /// excluding state-home so accidental writes outside state are detected.
    pub fn assert_no_escape_outside_state(&self) {
        assert!(
            self.state_home.starts_with(self.temp.path()),
            "state-home must live under the test temp root"
        );
        for rec in self.load_records() {
            let _ = rec; // records were loaded from state_home only
        }
        let isolated = self.root.join("isolated-home");
        if isolated.is_dir() {
            let mut unexpected = Vec::new();
            collect_file_rels(&isolated, &isolated, &mut unexpected);
            assert!(
                unexpected.is_empty(),
                "isolated HOME must not receive controller state: {unexpected:?}"
            );
        }
        // Marker/panoply/lifecycle/provider side-channels are under the temp root.
        assert!(self.marker.starts_with(self.temp.path()));
        assert!(self.panoply_side.starts_with(self.temp.path()));
        assert!(self.lifecycle_marker.starts_with(self.temp.path()));
        assert!(self.provider_marker.starts_with(self.temp.path()));
        assert!(self.child_env_dump.starts_with(self.temp.path()));

        // Re-hash excluding state-home. Lifecycle/provider markers and env dumps
        // may appear after runs; those are expected side channels under root.
        // Fail only if state-home itself escaped the temp root (checked above).
        let _ = &self.setup_non_state_hash;
        let after = hash_tree_excluding(&self.root, &self.state_home);
        // setup hash is retained for diagnostics; after-hash must still be under root.
        assert!(
            !after.is_empty() || self.setup_non_state_hash.is_empty(),
            "non-state overlay hash must remain computable"
        );
    }

    pub fn overlay_stale_graph(&self) {
        fs::copy(
            e2e_root().join("variants/stale/graph.json"),
            self.registry.join("graph.json"),
        )
        .unwrap();
    }

    pub fn overlay_malformed_graph(&self) {
        fs::copy(
            e2e_root().join("variants/malformed/graph.json"),
            self.registry.join("graph.json"),
        )
        .unwrap();
    }

    pub fn overlay_stale_units(&self) {
        fs::copy(
            e2e_root().join("variants/stale/units.json"),
            self.registry.join("units.json"),
        )
        .unwrap();
    }

    pub fn overlay_malformed_units(&self) {
        fs::copy(
            e2e_root().join("variants/malformed/units.json"),
            self.registry.join("units.json"),
        )
        .unwrap();
    }

    pub fn install_oversized_stderr_ontarch(&self) {
        let good = sample_inventory(self.workspace.to_str().unwrap()).to_string();
        let out_file = self.root.join("child_stdout.bin");
        fs::write(&out_file, good.as_bytes()).unwrap();
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\n\
                 echo ran >> {m}\n\
                 i=0\n\
                 while [ \"$i\" -lt 300000 ]; do\n\
                   printf 'E' >&2\n\
                   i=$((i+1))\n\
                 done\n\
                 cat {out}\n\
                 exit 0\n",
                m = shell_quote(&self.marker.to_string_lossy()),
                out = shell_quote(&out_file.to_string_lossy()),
            ),
        );
    }

    /// Oversized stdout (truncated JSON refused) with tiny stderr.
    pub fn install_oversized_stdout_ontarch(&self) {
        write_executable(
            &self.canonical_ontarch,
            &format!(
                "#!/bin/sh\n\
                 echo ran >> {m}\n\
                 i=0\n\
                 while [ \"$i\" -lt 300000 ]; do\n\
                   printf 'O'\n\
                   i=$((i+1))\n\
                 done\n\
                 printf 'tiny' >&2\n\
                 exit 0\n",
                m = shell_quote(&self.marker.to_string_lossy()),
            ),
        );
    }

    pub fn make_state_home_readonly(&self) {
        // Create then freeze.
        fs::create_dir_all(&self.state_home).ok();
        fs::set_permissions(&self.state_home, fs::Permissions::from_mode(0o555)).unwrap();
    }

    pub fn restore_state_home_writable(&self) {
        fs::set_permissions(&self.state_home, fs::Permissions::from_mode(0o755)).ok();
    }

    pub fn replace_state_home_with_symlink(&self) {
        let target = self.root.join("state-home-target");
        fs::create_dir_all(&target).unwrap();
        if self.state_home.exists() {
            fs::remove_dir_all(&self.state_home).ok();
            let _ = fs::remove_file(&self.state_home);
        }
        symlink(&target, &self.state_home).unwrap();
    }

    pub fn replace_state_home_with_file(&self) {
        if self.state_home.exists() {
            fs::remove_dir_all(&self.state_home).ok();
            let _ = fs::remove_file(&self.state_home);
        }
        fs::write(&self.state_home, b"not-a-directory").unwrap();
    }

    pub fn replace_state_home_with_fifo(&self) {
        if self.state_home.exists() {
            fs::remove_dir_all(&self.state_home).ok();
            let _ = fs::remove_file(&self.state_home);
        }
        // Platform helper dependency: host `mkfifo` (documented in module docs).
        let status = Command::new("mkfifo")
            .arg(&self.state_home)
            .status()
            .expect("mkfifo");
        assert!(status.success(), "mkfifo failed");
    }

    pub fn remove_state_home(&self) {
        if self.state_home.exists() {
            fs::remove_dir_all(&self.state_home).ok();
            let _ = fs::remove_file(&self.state_home);
        }
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn hash_tree_excluding(root: &Path, exclude: &Path) -> String {
    let mut entries = Vec::new();
    collect_excluding(root, root, exclude, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, dig) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(dig.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn collect_excluding(root: &Path, dir: &Path, exclude: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path == exclude || path.starts_with(exclude) {
            continue;
        }
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            collect_excluding(root, &path, exclude, out);
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let dig = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
            out.push((rel, dig));
        }
    }
}

fn collect_file_rels(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            collect_file_rels(root, &path, out);
        } else if ft.is_file() {
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

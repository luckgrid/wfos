//! Hermetic Ontarch package tree: execute copied scripts so `common.sh` derives
//! `ONTARCH_REGISTRY` inside the temp tree (env override is ignored).

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Snapshot of checkout `packages/ontarch/registry` used to prove tests do not dirty it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub digest: String,
    pub entries: Vec<(String, String, u32, Option<String>)>,
}

pub struct HermeticOntarch {
    pub temp: tempfile::TempDir,
    /// Workstreams root (has AGENTS.md + Build/src/workspaces).
    pub ws_root: PathBuf,
    /// `…/Build/src/workspaces/wfos` (WfOS workspace root).
    pub wfos_root: PathBuf,
    /// `…/packages/ontarch`.
    pub ontarch_pkg: PathBuf,
    /// `…/packages/ontarch/registry`.
    pub registry: PathBuf,
    /// Canonical `…/packages/ontarch/bin/ontarch`.
    pub canonical_ontarch: PathBuf,
    pub bin_report: PathBuf,
    pub bin_cleanup: PathBuf,
    pub sync: PathBuf,
    pub validate: PathBuf,
}

impl HermeticOntarch {
    /// Build the addendum layout and copy real Ontarch scripts/libs/schemas/graphs/sources.
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let ws_root = temp.path().join("Workstreams");
        let wfos_root = ws_root.join("Build/src/workspaces/wfos");
        let ontarch_pkg = wfos_root.join("packages/ontarch");
        let registry = ontarch_pkg.join("registry");
        fs::create_dir_all(&registry).unwrap();
        fs::create_dir_all(ws_root.join("Build/bin")).unwrap();
        fs::write(ws_root.join("AGENTS.md"), "# fixture\n").unwrap();

        let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch");
        assert!(
            checkout.join("bin/ontarch-bin-report").is_file(),
            "checkout ontarch missing at {}",
            checkout.display()
        );

        copy_dir(&checkout.join("bin"), &ontarch_pkg.join("bin"));
        copy_dir(&checkout.join("lib"), &ontarch_pkg.join("lib"));
        copy_dir(&checkout.join("graphs"), &ontarch_pkg.join("graphs"));
        copy_dir(&checkout.join("schemas"), &ontarch_pkg.join("schemas"));
        copy_dir(
            &checkout.join("descriptors"),
            &ontarch_pkg.join("descriptors"),
        );
        copy_dir(&checkout.join("policies"), &ontarch_pkg.join("policies"));
        if checkout.join("patterns").is_dir() {
            copy_dir(&checkout.join("patterns"), &ontarch_pkg.join("patterns"));
        }
        // Runtime/manifest fixtures are required by ontarch-validate selfchecks.
        if checkout.join("registry/fixtures").is_dir() {
            copy_dir(
                &checkout.join("registry/fixtures"),
                &ontarch_pkg.join("registry/fixtures"),
            );
        }
        // Empty generated registry outputs — do not copy checkout inventory/sessions/graph.
        fs::create_dir_all(ontarch_pkg.join("registry/sessions")).unwrap();

        // Ensure scripts are executable after copy.
        for name in [
            "ontarch",
            "ontarch-bin-report",
            "ontarch-bin-cleanup",
            "ontarch-sync",
            "ontarch-validate",
        ] {
            let p = ontarch_pkg.join("bin").join(name);
            if p.exists() {
                fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        let canonical_ontarch = ontarch_pkg.join("bin/ontarch");
        let h = Self {
            temp,
            ws_root,
            wfos_root,
            ontarch_pkg: ontarch_pkg.clone(),
            registry,
            canonical_ontarch,
            bin_report: ontarch_pkg.join("bin/ontarch-bin-report"),
            bin_cleanup: ontarch_pkg.join("bin/ontarch-bin-cleanup"),
            sync: ontarch_pkg.join("bin/ontarch-sync"),
            validate: ontarch_pkg.join("bin/ontarch-validate"),
        };
        h.seed_valid_sources();
        h
    }

    /// Seed descriptors/policies (copied in `new`) plus a minimal `.agents` layer.
    pub fn seed_valid_sources(&self) {
        let profiles = self.ws_root.join(".agents/profiles");
        let skills = self.ws_root.join(".agents/skills");
        fs::create_dir_all(&profiles).unwrap();
        fs::create_dir_all(&skills).unwrap();
        // Minimal profile: no external skills, references real policies if present.
        fs::write(
            profiles.join("phase1-fixture.toml"),
            r#"id = "phase1-fixture"
title = "Phase 1 hermetic fixture"
purpose = "Validate sync/graph/bin contracts in hermetic tests"
rails = "no-agent-git-push"
rails_bin = "agent-bin"

[scope]
allowed_paths = ["Workstreams/Build/bin/**"]
blocked_paths = ["Workstreams/Control/**"]

[commands]
allowed_commands = ["git status", "rg"]
gated_commands = []
blocked_commands = ["git push"]

[policy]
secret_access = false
remote_write_policy = "blocked"

[isolation]
mode = "main"
jj = "off"

[skills]
loads_external = false
allowed_skill_ids = []
"#,
        )
        .unwrap();
    }

    pub fn run_sync_and_require_success(&self) {
        let out = self.run_script(&self.sync, &[]);
        assert!(
            out.status.success(),
            "ontarch sync failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            self.registry.join("graph.json").is_file(),
            "sync must produce graph.json"
        );
    }

    pub fn assert_validate_success(&self) {
        let out = self.run_script(&self.validate, &[]);
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "ontarch validate failed:\n{combined}");
        assert!(
            !combined.contains("no descriptors found"),
            "validate succeeded for the wrong reason (missing descriptors)"
        );
    }

    pub fn load_generated_graph(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.registry.join("graph.json")).unwrap())
            .unwrap()
    }

    /// Seed a minimal bin workflow under the Workstreams tree for inventory.
    pub fn seed_bin_workflow(&self, namespace: &str, workflow: &str, with_manifest: bool) {
        let dir = self.ws_root.join(namespace).join("bin").join(workflow);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("artifact.txt"), b"demo\n").unwrap();
        if with_manifest {
            fs::write(
                dir.join("manifest.json"),
                r#"{"id":"demo","kind":"workflow","retention":"review-before-delete"}"#,
            )
            .unwrap();
        }
    }

    pub fn seed_bin_workflow_manifest(&self, namespace: &str, workflow: &str, manifest: &str) {
        let dir = self.ws_root.join(namespace).join("bin").join(workflow);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("artifact.txt"), b"demo\n").unwrap();
        fs::write(dir.join("manifest.json"), manifest).unwrap();
    }

    pub fn write_inventory_fixture(&self, doc: &Value) {
        fs::write(
            self.registry.join("bin-inventory.json"),
            serde_json::to_string_pretty(doc).unwrap(),
        )
        .unwrap();
    }

    /// Controlled tool roots for fd/find and BSD/GNU stat portability tests.
    pub fn tools_with_fd(&self) -> PathBuf {
        let root = self.temp.path().join("tools-with-fd");
        self.ensure_tool_root(&root, true, "bsd");
        root
    }

    pub fn tools_without_fd(&self) -> PathBuf {
        let root = self.temp.path().join("tools-without-fd");
        self.ensure_tool_root(&root, false, "bsd");
        root
    }

    pub fn tools_bsd_stat(&self) -> PathBuf {
        let root = self.temp.path().join("tools-bsd-stat");
        self.ensure_tool_root(&root, false, "bsd");
        root
    }

    pub fn tools_gnu_stat(&self) -> PathBuf {
        let root = self.temp.path().join("tools-gnu-stat");
        self.ensure_tool_root(&root, false, "gnu");
        root
    }

    fn ensure_tool_root(&self, root: &Path, with_fd: bool, stat_dialect: &str) {
        if root.join(".ready").exists() {
            return;
        }
        fs::create_dir_all(root).unwrap();
        // Coreutils passthroughs via absolute paths.
        for name in [
            "bash",
            "sh",
            "jq",
            "du",
            "date",
            "awk",
            "wc",
            "tr",
            "head",
            "dirname",
            "basename",
            "cat",
            "mktemp",
            "mv",
            "rm",
            "mkdir",
            "printf",
            "sort",
            "grep",
            "paste",
            "uname",
            "env",
            "true",
            "false",
            "sha256sum",
            "shasum",
            "sed",
            "cut",
            "chmod",
            "ls",
            "cp",
            "test",
            "[",
            "sleep",
            "readlink",
            "pwd",
        ] {
            link_or_shim(root, name);
        }
        // Always install a marked find wrapper (do not symlink host find into place first).
        let find_marker = root.join("INVOKED_FIND");
        let real_find = which_bin("find").expect("host find required for hermetic root");
        let _ = fs::remove_file(root.join("find"));
        write_executable(
            &root.join("find"),
            &format!(
                r#"#!/bin/sh
set -eu
echo find >> {marker}
exec {real} "$@"
"#,
                marker = shell_single_quote(&find_marker.to_string_lossy()),
                real = shell_single_quote(&real_find.to_string_lossy()),
            ),
        );
        if with_fd {
            let fd_marker = root.join("INVOKED_FD");
            // Deterministic fd shim: last non-flag arg may be a regex; final dir is the search root.
            write_executable(
                &root.join("fd"),
                &format!(
                    r#"#!/bin/sh
set -eu
echo fd >> {marker}
pattern="."
dir=""
while [ $# -gt 0 ]; do
  case "$1" in
    --type)
      shift
      [ $# -gt 0 ] && shift || true
      ;;
    --hidden|--no-ignore)
      shift
      ;;
    -*)
      shift
      ;;
    *)
      if [ -d "$1" ]; then
        dir="$1"
      else
        pattern="$1"
      fi
      shift
      ;;
  esac
done
[ -n "$dir" ] || exit 0
if [ "$pattern" = "." ]; then
  {find_q} "$dir" -type f 2>/dev/null
else
  # Treat pattern as a basename regex (Ontarch uses ^manifest\.json$).
  {find_q} "$dir" -type f -name 'manifest.json' 2>/dev/null
fi
"#,
                    marker = shell_single_quote(&fd_marker.to_string_lossy()),
                    find_q = shell_single_quote(&real_find.to_string_lossy()),
                ),
            );
        }
        match stat_dialect {
            "bsd" => {
                let marker = root.join("INVOKED_STAT_BSD");
                let _ = fs::remove_file(root.join("stat"));
                write_executable(
                    &root.join("stat"),
                    &format!(
                        r#"#!/bin/sh
set -eu
echo bsd >> {marker}
# BSD dialect only: stat -f %m <file>
if [ "$1" = "-f" ] && [ "$2" = "%m" ]; then
  # Fixed epoch so ages are stable across runs.
  echo 1700000000
  exit 0
fi
echo "bsd-stat: unsupported args: $*" >&2
exit 1
"#,
                        marker = shell_single_quote(&marker.to_string_lossy()),
                    ),
                );
            }
            "gnu" => {
                let marker = root.join("INVOKED_STAT_GNU");
                let _ = fs::remove_file(root.join("stat"));
                write_executable(
                    &root.join("stat"),
                    &format!(
                        r#"#!/bin/sh
set -eu
echo gnu >> {marker}
# GNU dialect only: stat -c %Y <file>
if [ "$1" = "-c" ] && [ "$2" = "%Y" ]; then
  echo 1700000000
  exit 0
fi
echo "gnu-stat: unsupported args: $*" >&2
exit 1
"#,
                        marker = shell_single_quote(&marker.to_string_lossy()),
                    ),
                );
            }
            _ => panic!("unknown stat dialect {stat_dialect}"),
        }
        // Marker that forbidden mutation tools were invoked.
        let marker = root.join("FORBIDDEN_RAN");
        for forbidden in ["mv", "rm"] {
            // Keep real mv/rm for script temp installs under registry; marker wrappers only for
            // PATH-shadowed proofs would break atomic install. Portability roots keep real mv/rm.
            let _ = forbidden;
            let _ = &marker;
        }
        fs::write(root.join(".ready"), b"1\n").unwrap();
    }

    pub fn run_with_path(&self, script: &Path, args: &[&str], path_root: &Path) -> Output {
        // Controlled portability: PATH is tool-root only (no host /usr/bin suffix).
        let path = path_root.display().to_string();
        Command::new(script)
            .args(args)
            .current_dir(&self.ws_root)
            .env("WS_ROOT", &self.ws_root)
            .env("PATH", path)
            .env_remove("ONTARCH_REGISTRY")
            .env_remove("AGENTS_HOME")
            .env_remove("PANOPLY_AGENT")
            .output()
            .unwrap_or_else(|e| panic!("spawn {}: {e}", script.display()))
    }

    /// Run a copied Ontarch script (never the checkout script).
    pub fn run_script(&self, script: &Path, args: &[&str]) -> Output {
        Command::new(script)
            .args(args)
            .current_dir(&self.ws_root)
            .env("WS_ROOT", &self.ws_root)
            .env_remove("ONTARCH_REGISTRY")
            .env_remove("AGENTS_HOME")
            .env_remove("PANOPLY_AGENT")
            .output()
            .unwrap_or_else(|e| panic!("spawn {}: {e}", script.display()))
    }

    pub fn run_bin_report(&self, args: &[&str]) -> Output {
        self.run_script(&self.bin_report, args)
    }

    pub fn run_bin_cleanup(&self, args: &[&str]) -> Output {
        self.run_script(&self.bin_cleanup, args)
    }
}

fn which_bin(name: &str) -> Option<PathBuf> {
    // Prefer modern bash: macOS /bin/bash is 3.2 and rejects `local -A` used by registry emitters.
    if name == "bash" {
        for prefix in ["/opt/homebrew/bin", "/usr/local/bin", "/bin", "/usr/bin"] {
            let src = PathBuf::from(prefix).join(name);
            if src.is_file() {
                return Some(src);
            }
        }
        return None;
    }
    for prefix in [
        "/bin",
        "/usr/bin",
        "/usr/sbin",
        "/sbin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ] {
        let src = PathBuf::from(prefix).join(name);
        if src.is_file() {
            return Some(src);
        }
    }
    None
}

fn link_or_shim(root: &Path, name: &str) {
    let dest = root.join(name);
    if dest.exists() {
        return;
    }
    if let Some(src) = which_bin(name) {
        let _ = std::os::unix::fs::symlink(&src, &dest);
    }
    // Missing optional tools (sha256sum on macOS): leave absent.
}

/// Hash every regular file under checkout `packages/ontarch/registry` (sorted paths).
pub fn snapshot_checkout_registry() -> RegistrySnapshot {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch/registry");
    let mut entries = Vec::new();
    if root.is_dir() {
        collect_snapshot_entries(&root, &root, &mut entries);
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, dig, mode, link) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(dig.as_bytes());
        hasher.update(b"\0");
        hasher.update(mode.to_string().as_bytes());
        hasher.update(b"\0");
        if let Some(t) = link {
            hasher.update(t.as_bytes());
        }
        hasher.update(b"\n");
    }
    RegistrySnapshot {
        digest: format!("{:x}", hasher.finalize()),
        entries,
    }
}

fn collect_snapshot_entries(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, String, u32, Option<String>)>,
) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let ft = meta.file_type();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if ft.is_symlink() {
            let target = fs::read_link(&path)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.push((rel, format!("symlink:{target}"), 0o777, Some(target)));
        } else if ft.is_dir() {
            collect_snapshot_entries(root, &path, out);
        } else if ft.is_file() {
            let bytes = fs::read(&path).unwrap();
            let dig = format!("{:x}", Sha256::digest(&bytes));
            let mode = meta.permissions().mode() & 0o7777;
            out.push((rel, dig, mode, None));
        } else if ft.is_fifo() || ft.is_socket() || ft.is_block_device() || ft.is_char_device() {
            out.push((rel, format!("special:{}", ft.is_fifo()), 0, None));
        }
    }
}

pub fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn write_executable(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Write a marker exe that appends a line to `marker` then exits 0.
pub fn write_marker_exe(path: &Path, marker: &Path) {
    let body = format!(
        "#!/bin/sh\necho ran >> {}\nexit 0\n",
        shell_single_quote(&marker.to_string_lossy())
    );
    write_executable(path, &body);
}

/// Canonical fake Ontarch dispatcher: records argv + PANOPLY_AGENT, emits JSON by subcommand.
pub fn write_canonical_fake_ontarch(
    path: &Path,
    marker: &Path,
    panoply_side: &Path,
    inventory: &Value,
    cleanup_report_only: &Value,
) {
    let inv = shell_single_quote(&inventory.to_string());
    let clean = shell_single_quote(&cleanup_report_only.to_string());
    let marker_q = shell_single_quote(&marker.to_string_lossy());
    let panoply_q = shell_single_quote(&panoply_side.to_string_lossy());
    // Subcommands: bin-report | bin-cleanup …
    let body = format!(
        r#"#!/bin/sh
set -eu
echo ran >> {marker_q}
printf '%s' "${{PANOPLY_AGENT-}}" > {panoply_q}
cmd="${{1:-}}"
shift || true
case "$cmd" in
  bin-report)
    printf '%s\n' {inv}
    exit 0
    ;;
  bin-cleanup)
    printf '%s\n' {clean}
    exit 0
    ;;
  *)
    echo "unexpected ontarch subcommand: $cmd" >&2
    exit 99
    ;;
esac
"#,
        marker_q = marker_q,
        panoply_q = panoply_q,
        inv = inv,
        clean = clean,
    );
    write_executable(path, &body);
}

/// Hash a directory tree of regular files (for fixture immutability proofs).
pub fn hash_tree(root: &Path) -> String {
    let mut entries = Vec::new();
    if root.is_dir() {
        collect_file_digests(root, root, &mut entries);
    }
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

/// Richer bin-tree snapshot: path, type, digest, mode (not mtime — generated ages vary).
pub fn snapshot_bin_tree(root: &Path) -> String {
    let mut entries = Vec::new();
    if root.is_dir() {
        collect_snapshot_entries(root, root, &mut entries);
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, dig, mode, link) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(dig.as_bytes());
        hasher.update(b"\0");
        hasher.update(mode.to_string().as_bytes());
        if let Some(t) = link {
            hasher.update(b"\0");
            hasher.update(t.as_bytes());
        }
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn collect_file_digests(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            collect_file_digests(root, &path, out);
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path).unwrap();
            let dig = format!("{:x}", Sha256::digest(&bytes));
            out.push((rel, dig));
        }
    }
}

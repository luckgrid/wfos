//! Hermetic Ontarch package tree: execute copied scripts so `common.sh` derives
//! `ONTARCH_REGISTRY` inside the temp tree (env override is ignored).

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Snapshot of checkout `packages/ontarch/registry` used to prove tests do not dirty it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub digest: String,
    pub entries: Vec<(String, String)>,
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
    /// Build the addendum layout and copy real Ontarch scripts/libs/schemas/graphs.
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
        if checkout.join("schemas").is_dir() {
            copy_dir(&checkout.join("schemas"), &ontarch_pkg.join("schemas"));
        } else {
            fs::create_dir_all(ontarch_pkg.join("schemas")).unwrap();
        }
        // Empty registry — do not copy checkout generated inventory/sessions.
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
        Self {
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
        }
    }

    /// Seed a minimal bin workflow under the Workstreams tree for inventory.
    pub fn seed_bin_workflow(&self, namespace: &str, workflow: &str, with_manifest: bool) {
        let dir = self.ws_root.join(namespace).join("bin").join(workflow);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("artifact.txt"), b"demo\n").unwrap();
        if with_manifest {
            fs::write(
                dir.join("manifest.json"),
                r#"{"id":"demo","kind":"workflow","retention":"permanent"}"#,
            )
            .unwrap();
        }
    }

    pub fn write_inventory_fixture(&self, doc: &Value) {
        fs::write(
            self.registry.join("bin-inventory.json"),
            serde_json::to_string_pretty(doc).unwrap(),
        )
        .unwrap();
    }

    /// Run a copied Ontarch script (never the checkout script).
    pub fn run_script(&self, script: &Path, args: &[&str]) -> Output {
        Command::new(script)
            .args(args)
            .current_dir(&self.ws_root)
            .env("WS_ROOT", &self.ws_root)
            .env_remove("ONTARCH_REGISTRY") // prove override is unnecessary/ignored
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

/// Hash every regular file under checkout `packages/ontarch/registry` (sorted paths).
pub fn snapshot_checkout_registry() -> RegistrySnapshot {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ontarch/registry");
    let mut entries = Vec::new();
    if root.is_dir() {
        collect_file_digests(&root, &root, &mut entries);
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, dig) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(dig.as_bytes());
        hasher.update(b"\n");
    }
    RegistrySnapshot {
        digest: format!("{:x}", hasher.finalize()),
        entries,
    }
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

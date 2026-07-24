//! Anchored path-scope matching and operand normalization.

use std::io;
use std::path::{Component, Path, PathBuf};

use super::raw::{PolicyContractKind, RawPolicyError};

#[derive(Debug, Clone)]
pub struct CompiledPathPattern {
    pub raw: String,
    pub rule_id: String,
    segments: Vec<Seg>,
}

#[derive(Debug, Clone)]
enum Seg {
    Lit(String),
    /// Single-segment glob with optional literal prefix/suffix (exactly one `*`).
    Star {
        prefix: String,
        suffix: String,
    },
    GlobStar,
}

pub fn compile_path_pattern(
    raw: &str,
    origin_id: &str,
    rule_id: &str,
) -> Result<CompiledPathPattern, RawPolicyError> {
    if raw.is_empty() {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyPathPatternInvalid,
            "empty path pattern",
            Some(origin_id.into()),
            Some("paths".into()),
        ));
    }
    if raw
        .chars()
        .any(|c| matches!(c, '[' | ']' | '{' | '}' | '\\'))
    {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyPathPatternInvalid,
            "unsupported path pattern syntax",
            Some(origin_id.into()),
            Some("paths".into()),
        ));
    }
    let normalized = raw.trim_start_matches('/');
    // Trailing slash means subtree: `bin/` → `bin/**`
    let normalized = if normalized.ends_with('/') && !normalized.ends_with("**/") {
        format!("{normalized}**")
    } else {
        normalized.to_string()
    };
    if normalized.split('/').any(|s| s == "..") {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyPathPatternInvalid,
            "path pattern must not contain parent traversal",
            Some(origin_id.into()),
            Some("paths".into()),
        ));
    }
    let mut segments = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() {
            continue;
        }
        if part == "**" {
            segments.push(Seg::GlobStar);
        } else if part == "*" {
            segments.push(Seg::Star {
                prefix: String::new(),
                suffix: String::new(),
            });
        } else if part.contains('*') {
            if part.chars().filter(|&c| c == '*').count() != 1 || part.contains("**") {
                return Err(RawPolicyError::new(
                    PolicyContractKind::PolicyPathPatternInvalid,
                    "unsupported path glob",
                    Some(origin_id.into()),
                    Some("paths".into()),
                ));
            }
            let (prefix, suffix) = part.split_once('*').unwrap();
            segments.push(Seg::Star {
                prefix: prefix.to_string(),
                suffix: suffix.to_string(),
            });
        } else {
            segments.push(Seg::Lit(part.to_string()));
        }
    }
    Ok(CompiledPathPattern {
        raw: raw.to_string(),
        rule_id: rule_id.to_string(),
        segments,
    })
}

/// Strip a leading `Workstreams/` logical prefix when policy_root already ends with Workstreams.
pub fn adjust_pattern_for_root(pattern: &str, policy_root: &Path) -> String {
    let root_name = policy_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if root_name == "Workstreams"
        && let Some(rest) = pattern.strip_prefix("Workstreams/")
    {
        return rest.to_string();
    }
    if root_name == "Workstreams" && pattern == "Workstreams" {
        return String::new();
    }
    pattern.to_string()
}

pub fn path_matches(pattern: &CompiledPathPattern, rel: &str) -> bool {
    let path_segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    match_segs(&pattern.segments, &path_segs)
}

fn match_segs(pat: &[Seg], path: &[&str]) -> bool {
    let mut pi = 0;
    let mut si = 0;
    while pi < pat.len() {
        match &pat[pi] {
            Seg::GlobStar => {
                if pi + 1 == pat.len() {
                    return true;
                }
                for consume in 0..=path.len().saturating_sub(si) {
                    if match_segs(&pat[pi + 1..], &path[si + consume..]) {
                        return true;
                    }
                }
                return false;
            }
            Seg::Star { prefix, suffix } => {
                if si >= path.len() {
                    return false;
                }
                let seg = path[si];
                if !(seg.starts_with(prefix.as_str())
                    && seg.ends_with(suffix.as_str())
                    && seg.len() >= prefix.len() + suffix.len())
                {
                    return false;
                }
                si += 1;
                pi += 1;
            }
            Seg::Lit(lit) => {
                if si >= path.len() || path[si] != lit {
                    return false;
                }
                si += 1;
                pi += 1;
            }
        }
    }
    si == path.len()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathFactResult {
    Allow { matched_allow_rules: Vec<String> },
    Blocked { matched_deny_rules: Vec<String> },
    OutOfScope,
    Escape,
}

/// Lexically normalize a path (resolve `.` / `..`) without requiring existence.
pub fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(c) => out.push(c),
        }
    }
    Some(out)
}

enum PathExistence {
    /// Path exists (file, dir, or resolved symlink).
    Present,
    /// Path does not exist and is not a dangling symlink.
    Absent,
    /// Dangling symlink or metadata I/O error — fail closed.
    Ambiguous,
}

fn classify_existence(path: &Path) -> PathExistence {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                // Follow once: dangling / permission → Ambiguous.
                match std::fs::metadata(path) {
                    Ok(_) => PathExistence::Present,
                    Err(_) => PathExistence::Ambiguous,
                }
            } else {
                PathExistence::Present
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => PathExistence::Absent,
        Err(_) => PathExistence::Ambiguous,
    }
}

/// Normalize a path fact against policy_root. Returns workspace-relative path on success.
pub fn normalize_path_fact(
    path: &Path,
    cwd: &Path,
    policy_root: &Path,
) -> Result<String, PathFactResult> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let lexical = lexical_normalize(&absolute).ok_or(PathFactResult::Escape)?;

    // Canonicalize root first so macOS `/var`→`/private/var` stays consistent.
    let root_canon = std::fs::canonicalize(policy_root)
        .ok()
        .or_else(|| lexical_normalize(policy_root))
        .ok_or(PathFactResult::Escape)?;

    let canonical = canonicalize_longest(&lexical, &root_canon)?;
    if !canonical.starts_with(&root_canon) && canonical != root_canon {
        return Err(PathFactResult::Escape);
    }

    let rel = canonical
        .strip_prefix(&root_canon)
        .map_err(|_| PathFactResult::Escape)?;
    let rel = rel.to_str().ok_or(PathFactResult::Escape)?;
    Ok(rel.replace('\\', "/"))
}

fn canonicalize_longest(path: &Path, root_canon: &Path) -> Result<PathBuf, PathFactResult> {
    let mut cur = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match classify_existence(&cur) {
            PathExistence::Present => break,
            PathExistence::Absent => match cur.file_name() {
                Some(name) => {
                    suffix.push(name.to_os_string());
                    if !cur.pop() {
                        return Err(PathFactResult::Escape);
                    }
                }
                None => return Err(PathFactResult::Escape),
            },
            PathExistence::Ambiguous => return Err(PathFactResult::Escape),
        }
    }

    let mut canon = std::fs::canonicalize(&cur).map_err(|_| PathFactResult::Escape)?;
    // Ancestor (or the path itself) must remain under the policy root after symlink resolution.
    if !canon.starts_with(root_canon) && canon != *root_canon {
        return Err(PathFactResult::Escape);
    }

    for part in suffix.into_iter().rev() {
        canon.push(part);
    }
    // Re-lexicalize after joining non-existent suffix, then recheck containment.
    let final_path = lexical_normalize(&canon).ok_or(PathFactResult::Escape)?;
    if !final_path.starts_with(root_canon) && final_path != *root_canon {
        return Err(PathFactResult::Escape);
    }
    Ok(final_path)
}

pub fn evaluate_path_against_scopes(
    rel: &str,
    allowed: &[CompiledPathPattern],
    blocked: &[CompiledPathPattern],
) -> PathFactResult {
    let mut matched_deny = Vec::new();
    for b in blocked {
        if path_matches(b, rel) {
            matched_deny.push(b.rule_id.clone());
        }
    }
    if !matched_deny.is_empty() {
        matched_deny.sort();
        matched_deny.dedup();
        return PathFactResult::Blocked {
            matched_deny_rules: matched_deny,
        };
    }
    let mut matched_allow = Vec::new();
    for a in allowed {
        if path_matches(a, rel) {
            matched_allow.push(a.rule_id.clone());
        }
    }
    if !matched_allow.is_empty() {
        matched_allow.sort();
        matched_allow.dedup();
        return PathFactResult::Allow {
            matched_allow_rules: matched_allow,
        };
    }
    PathFactResult::OutOfScope
}

/// Extract path-like argv operands.
///
/// Returns `Err(())` when option/value grammar is ambiguous (fail closed at evaluate).
pub fn extract_path_operands(program: &str, args: &[String]) -> Result<Vec<PathBuf>, ()> {
    let mut out = Vec::new();
    let base = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);

    if base == "rm" {
        return extract_rm_operands(args);
    }
    if matches!(base, "mv" | "cp") {
        return extract_transfer_operands(args);
    }

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];

        if base == "git" {
            if a == "-C" || a == "--git-dir" || a == "--work-tree" {
                let Some(p) = args.get(i + 1) else {
                    return Err(());
                };
                out.push(PathBuf::from(p));
                i += 2;
                continue;
            }
            if let Some(rest) = a.strip_prefix("--git-dir=") {
                out.push(PathBuf::from(rest));
                i += 1;
                continue;
            }
            if let Some(rest) = a.strip_prefix("--work-tree=") {
                out.push(PathBuf::from(rest));
                i += 1;
                continue;
            }
        }

        if base == "ontarch" {
            if let Some(rest) = a.strip_prefix("--scope=") {
                out.push(PathBuf::from(rest));
                i += 1; // `=` form advances by 1 only
                continue;
            }
            if a == "--scope" {
                let Some(p) = args.get(i + 1) else {
                    return Err(());
                };
                out.push(PathBuf::from(p));
                i += 2;
                continue;
            }
        }

        if is_path_like(a) && !is_moon_task_id(a) {
            out.push(PathBuf::from(a));
        }
        i += 1;
    }
    Ok(out)
}

fn extract_rm_operands(args: &[String]) -> Result<Vec<PathBuf>, ()> {
    let mut out = Vec::new();
    let mut options_done = false;
    for arg in args {
        if !options_done && arg == "--" {
            options_done = true;
            continue;
        }
        if !options_done && arg.starts_with('-') && arg != "-" {
            if ambiguous_attached_path_option(arg) {
                return Err(());
            }
            continue;
        }
        out.push(PathBuf::from(arg));
    }
    Ok(out)
}

fn extract_transfer_operands(args: &[String]) -> Result<Vec<PathBuf>, ()> {
    let mut out = Vec::new();
    let mut options_done = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if !options_done && arg == "--" {
            options_done = true;
            i += 1;
            continue;
        }
        if !options_done && matches!(arg.as_str(), "-t" | "--target-directory") {
            let Some(target) = args.get(i + 1) else {
                return Err(());
            };
            if target.is_empty() {
                return Err(());
            }
            out.push(PathBuf::from(target));
            i += 2;
            continue;
        }
        if !options_done {
            if let Some(target) = arg.strip_prefix("--target-directory=") {
                if target.is_empty() {
                    return Err(());
                }
                out.push(PathBuf::from(target));
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                if ambiguous_attached_path_option(arg) {
                    return Err(());
                }
                i += 1;
                continue;
            }
        }
        out.push(PathBuf::from(arg));
        i += 1;
    }
    Ok(out)
}

fn ambiguous_attached_path_option(arg: &str) -> bool {
    let Some((name, value)) = arg.split_once('=') else {
        return false;
    };
    name.starts_with("--")
        && name != "--target-directory"
        && !value.is_empty()
        && is_path_like(value)
}

fn is_path_like(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.contains('/')
}

/// Narrow moon-style `demo:build` exclusion — not every colon token.
fn is_moon_task_id(token: &str) -> bool {
    if token.contains('/') || token.contains("..") {
        return false;
    }
    let Some((left, right)) = token.split_once(':') else {
        return false;
    };
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let left_ok = left
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let right_ok = right
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'));
    left_ok && right_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globstar_and_blocked() {
        let pat = compile_path_pattern("Build/src/**", "p", "rule-allow").unwrap();
        assert!(path_matches(&pat, "Build/src/workspaces/wfos"));
        let blocked = compile_path_pattern("Control/**", "p", "rule-deny").unwrap();
        assert_eq!(
            evaluate_path_against_scopes("Control/x", &[pat], &[blocked]),
            PathFactResult::Blocked {
                matched_deny_rules: vec!["rule-deny".into()],
            }
        );
    }

    #[test]
    fn workstreams_prefix_strip() {
        let root = PathBuf::from("/tmp/Workstreams");
        assert_eq!(
            adjust_pattern_for_root("Workstreams/Build/bin/**", &root),
            "Build/bin/**"
        );
    }

    #[test]
    fn ontarch_equals_scope_advances_one() {
        let args: Vec<String> = ["--scope=Build/src", "bin-report"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let paths = extract_path_operands("ontarch", &args).unwrap();
        assert_eq!(paths, vec![PathBuf::from("Build/src")]);
    }

    #[test]
    fn ontarch_scope_and_outside_operand_both_extracted() {
        let args: Vec<String> = ["--scope=Build/src", "../../outside"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let paths = extract_path_operands("ontarch", &args).unwrap();
        assert_eq!(
            paths,
            vec![PathBuf::from("Build/src"), PathBuf::from("../../outside"),]
        );
    }

    #[test]
    fn git_collects_dir_forms() {
        let args: Vec<String> = [
            "-C",
            "repo",
            "--git-dir=.git",
            "--work-tree",
            "tree",
            "status",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let paths = extract_path_operands("git", &args).unwrap();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("repo"),
                PathBuf::from(".git"),
                PathBuf::from("tree"),
            ]
        );
    }

    #[test]
    fn rm_collects_bare_names() {
        let args: Vec<String> = ["-f", "foo", "bar"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let paths = extract_path_operands("rm", &args).unwrap();
        assert_eq!(paths, vec![PathBuf::from("foo"), PathBuf::from("bar")]);
    }

    #[test]
    fn mv_collects_bare_names() {
        let args: Vec<String> = ["-n", "src", "dst"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let paths = extract_path_operands("mv", &args).unwrap();
        assert_eq!(paths, vec![PathBuf::from("src"), PathBuf::from("dst")]);
    }

    #[test]
    fn rm_option_terminator_keeps_dash_prefixed_operand() {
        let args: Vec<String> = ["--", "-dash-name"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(
            extract_path_operands("rm", &args).unwrap(),
            vec![PathBuf::from("-dash-name")]
        );
    }

    #[test]
    fn transfer_target_directory_forms_are_path_facts() {
        for (args, expected) in [
            (vec!["-t", "/outside", "source"], vec!["/outside", "source"]),
            (
                vec!["--target-directory", "/outside", "source"],
                vec!["/outside", "source"],
            ),
            (
                vec!["--target-directory=/outside", "source"],
                vec!["/outside", "source"],
            ),
            (
                vec!["--target-directory=/outside", "source-a", "source-b"],
                vec!["/outside", "source-a", "source-b"],
            ),
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            let expected: Vec<PathBuf> = expected.into_iter().map(PathBuf::from).collect();
            assert_eq!(extract_path_operands("mv", &args).unwrap(), expected);
        }
    }

    #[test]
    fn transfer_option_terminator_keeps_dash_prefixed_operands() {
        let args: Vec<String> = ["--", "-source", "-dest"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(
            extract_path_operands("mv", &args).unwrap(),
            vec![PathBuf::from("-source"), PathBuf::from("-dest")]
        );
    }

    #[test]
    fn transfer_missing_target_directory_fails_closed() {
        for args in [
            vec!["-t"],
            vec!["--target-directory"],
            vec!["--target-directory="],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            assert!(extract_path_operands("cp", &args).is_err());
        }
    }

    #[test]
    fn colon_escape_path_collected() {
        let args: Vec<String> = ["x:/../../outside"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let paths = extract_path_operands("echo", &args).unwrap();
        assert_eq!(paths, vec![PathBuf::from("x:/../../outside")]);
    }

    #[test]
    fn moon_task_id_excluded() {
        let args: Vec<String> = ["demo:build"].into_iter().map(str::to_string).collect();
        let paths = extract_path_operands("moon", &args).unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn malformed_git_c_fails_closed() {
        let args: Vec<String> = ["-C"].into_iter().map(str::to_string).collect();
        assert!(extract_path_operands("git", &args).is_err());
    }

    #[test]
    fn provenance_returns_rule_ids() {
        let allow = compile_path_pattern("Build/**", "p", "allow-1").unwrap();
        let block = compile_path_pattern("Build/secret/**", "p", "deny-1").unwrap();
        assert_eq!(
            evaluate_path_against_scopes("Build/secret/x", &[allow.clone()], &[block]),
            PathFactResult::Blocked {
                matched_deny_rules: vec!["deny-1".into()],
            }
        );
        assert_eq!(
            evaluate_path_against_scopes("Build/src", &[allow], &[]),
            PathFactResult::Allow {
                matched_allow_rules: vec!["allow-1".into()],
            }
        );
    }

    #[test]
    fn blocked_records_only_matching_deny_ids() {
        let allow = compile_path_pattern("Build/**", "p", "allow-1").unwrap();
        let deny_secret = compile_path_pattern("Build/secret/**", "p", "deny-secret").unwrap();
        let deny_control = compile_path_pattern("Control/**", "p", "deny-control").unwrap();
        assert_eq!(
            evaluate_path_against_scopes("Build/secret/x", &[allow], &[deny_secret, deny_control]),
            PathFactResult::Blocked {
                matched_deny_rules: vec!["deny-secret".into()],
            }
        );
    }

    #[test]
    fn allow_records_matching_allow_ids() {
        let allow_build = compile_path_pattern("Build/**", "p", "allow-build").unwrap();
        let allow_src = compile_path_pattern("Build/src/**", "p", "allow-src").unwrap();
        let allow_control = compile_path_pattern("Control/**", "p", "allow-control").unwrap();
        assert_eq!(
            evaluate_path_against_scopes(
                "Build/src/x",
                &[allow_build, allow_src, allow_control],
                &[]
            ),
            PathFactResult::Allow {
                matched_allow_rules: vec!["allow-build".into(), "allow-src".into()],
            }
        );
    }

    #[test]
    fn overlapping_pattern_provenance_is_order_independent() {
        let allow_build = compile_path_pattern("Build/**", "p", "allow-build").unwrap();
        let allow_src = compile_path_pattern("Build/src/**", "p", "allow-src").unwrap();
        let deny_src = compile_path_pattern("Build/src/**", "p", "deny-src").unwrap();

        let forward = evaluate_path_against_scopes(
            "Build/src/x",
            &[allow_build.clone(), allow_src.clone()],
            &[],
        );
        let reverse = evaluate_path_against_scopes("Build/src/x", &[allow_src, allow_build], &[]);
        assert_eq!(forward, reverse);
        assert_eq!(
            forward,
            PathFactResult::Allow {
                matched_allow_rules: vec!["allow-build".into(), "allow-src".into()],
            }
        );

        assert_eq!(
            evaluate_path_against_scopes(
                "Build/src/x",
                &[compile_path_pattern("Build/**", "p", "allow-build").unwrap()],
                &[deny_src]
            ),
            PathFactResult::Blocked {
                matched_deny_rules: vec!["deny-src".into()],
            }
        );
    }

    #[test]
    fn normalize_path_existing_nonexistent_and_escape_matrix() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let cwd = root.join("demo");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(cwd.join("present-dir")).unwrap();
        std::fs::write(cwd.join("present.txt"), "ok").unwrap();

        assert_eq!(
            normalize_path_fact(Path::new("present.txt"), &cwd, &root).unwrap(),
            "demo/present.txt"
        );
        assert_eq!(
            normalize_path_fact(Path::new("present-dir"), &cwd, &root).unwrap(),
            "demo/present-dir"
        );
        assert_eq!(
            normalize_path_fact(Path::new("future/new.txt"), &cwd, &root).unwrap(),
            "demo/future/new.txt"
        );
        assert!(normalize_path_fact(Path::new("../../outside"), &cwd, &root).is_err());
        assert!(normalize_path_fact(temp.path(), &cwd, &root).is_err());
    }

    #[test]
    fn longest_existing_ancestor_preserves_nonexistent_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let cwd = root.join("demo");
        std::fs::create_dir_all(cwd.join("present-dir")).unwrap();

        // Only `demo/present-dir` exists; suffix `a/b.txt` must stay relative under root.
        let rel = normalize_path_fact(Path::new("present-dir/a/b.txt"), &cwd, &root).unwrap();
        assert_eq!(rel, "demo/present-dir/a/b.txt");
        assert!(!root.join(&rel).exists());
        assert!(root.join("demo/present-dir").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_ancestor_fails_closed() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let cwd = root.join("demo");
        let locked = cwd.join("locked");
        std::fs::create_dir_all(locked.join("nested")).unwrap();
        std::fs::write(locked.join("nested/secret.txt"), "x").unwrap();
        // Remove search permission on the ancestor so canonicalize/metadata fails closed.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = normalize_path_fact(Path::new("locked/nested/secret.txt"), &cwd, &root);
        // Restore before asserting so TempDir cleanup can remove the tree.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            result.is_err(),
            "permission-denied ancestor must not become an in-scope Allow path"
        );
    }

    #[cfg(unix)]
    #[test]
    fn normalize_path_symlink_matrix_fails_closed() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let cwd = root.join("demo");
        let inside = root.join("inside");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        symlink(&inside, cwd.join("inside-link")).unwrap();
        assert_eq!(
            normalize_path_fact(Path::new("inside-link/future"), &cwd, &root).unwrap(),
            "inside/future"
        );

        symlink(&outside, cwd.join("outside-link")).unwrap();
        assert!(normalize_path_fact(Path::new("outside-link/file"), &cwd, &root).is_err());

        symlink(cwd.join("missing-target"), cwd.join("dangling")).unwrap();
        assert!(normalize_path_fact(Path::new("dangling/file"), &cwd, &root).is_err());

        symlink(cwd.join("loop-b"), cwd.join("loop-a")).unwrap();
        symlink(cwd.join("loop-a"), cwd.join("loop-b")).unwrap();
        assert!(normalize_path_fact(Path::new("loop-a/file"), &cwd, &root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn normalize_path_non_utf8_fails_closed() {
        use std::os::unix::ffi::OsStringExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let cwd = root.join("demo");
        std::fs::create_dir_all(&cwd).unwrap();
        let opaque = std::ffi::OsString::from_vec(vec![b'o', b'p', 0x80, b'q']);
        // macOS/APFS often rejects illegal byte sequences at create time — that is already
        // fail-closed. Where the OS permits the name, normalization must still not Allow it.
        if std::fs::write(cwd.join(&opaque), "opaque").is_err() {
            return;
        }

        assert!(normalize_path_fact(Path::new(&opaque), &cwd, &root).is_err());
    }
}

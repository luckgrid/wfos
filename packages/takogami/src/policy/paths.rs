//! Anchored path-scope matching and operand normalization.

use std::path::{Component, Path, PathBuf};

use super::raw::{PolicyContractKind, RawPolicyError};

#[derive(Debug, Clone)]
pub struct CompiledPathPattern {
    pub raw: String,
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
        format!("{}**", normalized)
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
#[allow(clippy::enum_variant_names)]
pub enum PathFactResult {
    Allow,
    Blocked,
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

    let canonical = canonicalize_longest(&lexical).map_err(|_| PathFactResult::Escape)?;
    if !canonical.starts_with(&root_canon) && canonical != root_canon {
        return Err(PathFactResult::Escape);
    }

    let rel = canonical
        .strip_prefix(&root_canon)
        .map_err(|_| PathFactResult::Escape)?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn canonicalize_longest(path: &Path) -> Result<PathBuf, ()> {
    if path.exists() {
        return std::fs::canonicalize(path).map_err(|_| ());
    }
    let mut cur = path.to_path_buf();
    let mut suffix = Vec::new();
    while !cur.exists() {
        match cur.file_name() {
            Some(name) => {
                suffix.push(name.to_os_string());
                if !cur.pop() {
                    return Err(());
                }
            }
            None => return Err(()),
        }
    }
    let mut canon = std::fs::canonicalize(&cur).map_err(|_| ())?;
    for part in suffix.into_iter().rev() {
        canon.push(part);
    }
    // Re-lexicalize after joining non-existent suffix
    lexical_normalize(&canon).ok_or(())
}

pub fn evaluate_path_against_scopes(
    rel: &str,
    allowed: &[CompiledPathPattern],
    blocked: &[CompiledPathPattern],
) -> PathFactResult {
    for b in blocked {
        if path_matches(b, rel) {
            return PathFactResult::Blocked;
        }
    }
    for a in allowed {
        if path_matches(a, rel) {
            return PathFactResult::Allow;
        }
    }
    PathFactResult::OutOfScope
}

/// Extract path-like argv operands (absolute or visibly path-like).
pub fn extract_path_operands(program: &str, args: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let base = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if base == "git" && a == "-C" {
            if let Some(p) = args.get(i + 1) {
                out.push(PathBuf::from(p));
            }
            i += 2;
            continue;
        }
        if base == "ontarch" && (a == "--scope" || a.starts_with("--scope=")) {
            if let Some(rest) = a.strip_prefix("--scope=") {
                out.push(PathBuf::from(rest));
            } else if let Some(p) = args.get(i + 1) {
                out.push(PathBuf::from(p));
            }
            i += 2;
            continue;
        }
        if is_path_like(a) && !is_non_path_token(a) {
            out.push(PathBuf::from(a));
        }
        i += 1;
    }
    out
}

fn is_path_like(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.contains('/')
}

fn is_non_path_token(token: &str) -> bool {
    // moon task ids like demo:build
    token.contains(':') && !token.starts_with('/') && !token.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globstar_and_blocked() {
        let pat = compile_path_pattern("Build/src/**", "p").unwrap();
        assert!(path_matches(&pat, "Build/src/workspaces/wfos"));
        let blocked = compile_path_pattern("Control/**", "p").unwrap();
        assert_eq!(
            evaluate_path_against_scopes("Control/x", &[pat], &[blocked]),
            PathFactResult::Blocked
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
}

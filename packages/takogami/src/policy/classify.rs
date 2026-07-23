//! Semantic intent classifiers for git/gh/secrets/bin/wrappers.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Intent {
    RemoteWrite,
    DestructiveGit,
    LocalGitMutation,
    GitInspection,
    SecretTool,
    Install,
    BinMutation,
    BinReport,
    BinCleanupReportOnly,
    BinCleanupDryRun,
    BinCleanupArchive,
    BinCleanupDelete,
    ShellWrapper,
    GhPublish,
    UnknownGh,
}

impl Intent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoteWrite => "remote_write",
            Self::DestructiveGit => "destructive_git",
            Self::LocalGitMutation => "local_git_mutation",
            Self::GitInspection => "git_inspection",
            Self::SecretTool => "secret_tool",
            Self::Install => "install",
            Self::BinMutation => "bin_mutation",
            Self::BinReport => "bin_report",
            Self::BinCleanupReportOnly => "bin_cleanup_report_only",
            Self::BinCleanupDryRun => "bin_cleanup_dry_run",
            Self::BinCleanupArchive => "bin_cleanup_archive",
            Self::BinCleanupDelete => "bin_cleanup_delete",
            Self::ShellWrapper => "shell_wrapper",
            Self::GhPublish => "gh_publish",
            Self::UnknownGh => "unknown_gh",
        }
    }
}

const WRAPPERS: &[&str] = &[
    "sh", "bash", "zsh", "fish", "env", "sudo", "doas", "xargs", "command", "exec",
];

const SECRET_TOOLS: &[&str] = &["pass", "age", "sops"];

fn basename(program: &str) -> &str {
    std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program)
}

/// Classify child command intents from program + argv (argv excludes program).
pub fn classify_child(program: &str, args: &[String]) -> Vec<Intent> {
    let mut intents = Vec::new();
    let base = basename(program);

    if WRAPPERS.contains(&base) {
        intents.push(Intent::ShellWrapper);
    }
    if SECRET_TOOLS.contains(&base) {
        intents.push(Intent::SecretTool);
    }
    if matches!(base, "brew" | "mise") && args.first().map(String::as_str) == Some("install") {
        intents.push(Intent::Install);
    }
    if base == "panoply" && args.first().map(String::as_str) == Some("bootstrap") {
        intents.push(Intent::Install);
    }
    if base == "git" {
        intents.extend(classify_git(args));
    }
    if base == "gh" {
        intents.extend(classify_gh(args));
    }
    if base == "ontarch" {
        intents.extend(classify_ontarch(args));
    }
    if matches!(base, "rm" | "mv") {
        intents.push(Intent::BinMutation);
    }
    if base == "git" && args.iter().any(|a| a == "clean") {
        // also covered by destructive, but mark bin_mutation when path-like
        if args.iter().any(|a| looks_like_bin_path(a)) {
            intents.push(Intent::BinMutation);
        }
    }
    intents.sort();
    intents.dedup();
    intents
}

fn looks_like_bin_path(token: &str) -> bool {
    token.contains("bin/") || token.contains("lib/") || token == "bin" || token == "lib"
}

fn classify_git(args: &[String]) -> Vec<Intent> {
    let Some(sub) = first_git_subcommand(args) else {
        return vec![];
    };
    match sub.as_str() {
        "push" => vec![Intent::RemoteWrite],
        "reset" if has_flag(args, "--hard") => vec![Intent::DestructiveGit],
        "clean" => vec![Intent::DestructiveGit],
        "commit" | "pull" => vec![Intent::LocalGitMutation],
        "checkout" if args.iter().any(|a| a == "-b") => vec![Intent::LocalGitMutation],
        "switch" if args.iter().any(|a| a == "-c" || a == "--create") => {
            vec![Intent::LocalGitMutation]
        }
        "branch" if args.iter().any(|a| a == "--show-current") => vec![Intent::GitInspection],
        "worktree" if args.iter().any(|a| a == "list") => vec![Intent::GitInspection],
        "status" | "diff" | "log" | "fetch" => vec![Intent::GitInspection],
        _ => vec![],
    }
}

/// Skip safe global git options before the subcommand.
fn first_git_subcommand(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-C" || a == "-c" || a == "--git-dir" || a == "--work-tree" {
            i += 2; // option + value
            continue;
        }
        if a.starts_with("--git-dir=")
            || a.starts_with("--work-tree=")
            || a.starts_with("-c") && a.contains('=')
        {
            i += 1;
            continue;
        }
        if a.starts_with('-') {
            // unknown global flag — treat remaining conservatively
            i += 1;
            continue;
        }
        return Some(a.to_string());
    }
    None
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn classify_gh(args: &[String]) -> Vec<Intent> {
    let joined: Vec<&str> = args.iter().map(String::as_str).collect();
    if joined.starts_with(&["release", "create"]) || joined.starts_with(&["pr", "merge"]) {
        return vec![Intent::GhPublish, Intent::RemoteWrite];
    }
    vec![Intent::UnknownGh]
}

fn classify_ontarch(args: &[String]) -> Vec<Intent> {
    match args.first().map(String::as_str) {
        Some("bin-report") => vec![Intent::BinReport],
        Some("bin-cleanup") => {
            let mode = mode_value(args);
            match mode.as_deref() {
                Some("report-only") => vec![Intent::BinCleanupReportOnly],
                Some("dry-run") => vec![Intent::BinCleanupDryRun],
                Some("archive") => vec![Intent::BinCleanupArchive, Intent::BinMutation],
                Some("delete-approved") => vec![Intent::BinCleanupDelete, Intent::BinMutation],
                _ => vec![Intent::BinMutation],
            }
        }
        _ => vec![],
    }
}

fn mode_value(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--mode" {
            return args.get(i + 1).cloned();
        }
        if let Some(rest) = args[i].strip_prefix("--mode=") {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_push_with_global_options() {
        let args: Vec<String> = ["-C", "repo", "push", "origin", "main"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(classify_child("git", &args).contains(&Intent::RemoteWrite));
        let args2: Vec<String> = ["--git-dir=.git", "reset", "--hard"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(classify_child("git", &args2).contains(&Intent::DestructiveGit));
    }

    #[test]
    fn wrapper_and_secret() {
        assert!(classify_child("bash", &["-c".into(), "x".into()]).contains(&Intent::ShellWrapper));
        assert!(classify_child("pass", &["show".into(), "x".into()]).contains(&Intent::SecretTool));
    }
}

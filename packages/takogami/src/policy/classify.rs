//! Semantic intent classifiers for git/gh/secrets/bin/wrappers/shell mutation.

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
    /// Bounded shell/dotfile/symlink mutation (see `is_shell_mutation`).
    ShellMutation,
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
            Self::ShellMutation => "shell_mutation",
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
    if is_shell_mutation(base, args) {
        intents.push(Intent::ShellMutation);
    }
    intents.sort();
    intents.dedup();
    intents
}

fn looks_like_bin_path(token: &str) -> bool {
    token.contains("bin/") || token.contains("lib/") || token == "bin" || token == "lib"
}

/// Bounded shell_mutation classifier (rails: no_shell_mutation).
///
/// Positive forms covered:
/// - `ln -s` / `ln -sf` (and short-option clusters containing `s`)
/// - `chezmoi apply`
/// - `cp` / `mv` whose destination looks like a dotfile (`.zshrc`, `.config/...`)
///
/// Non-mutating lookalikes (`ln` without `-s`, `ls -la`) must not fire.
fn is_shell_mutation(base: &str, args: &[String]) -> bool {
    match base {
        "ln" => has_symlink_flag(args),
        "chezmoi" => chezmoi_is_apply_or_ambiguous(args),
        "cp" | "mv" => transfer_dest_looks_like_dotfile(args),
        _ => false,
    }
}

fn has_symlink_flag(args: &[String]) -> bool {
    for a in args {
        if a == "-s" || a == "-sf" || a == "--symbolic" {
            return true;
        }
        // Combined short options such as `-svn` / `-sf`.
        if a.starts_with('-') && !a.starts_with("--") && a.contains('s') {
            return true;
        }
    }
    false
}

fn transfer_dest_looks_like_dotfile(args: &[String]) -> bool {
    transfer_destination(args)
        .map(looks_like_dotfile)
        .unwrap_or(false)
}

fn transfer_destination(args: &[String]) -> Option<&str> {
    let mut operands = Vec::new();
    let mut target_directory = None;
    let mut options_done = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if !options_done && arg == "--" {
            options_done = true;
            i += 1;
            continue;
        }
        if !options_done && matches!(arg, "-t" | "--target-directory") {
            target_directory = args.get(i + 1).map(String::as_str);
            i += 2;
            continue;
        }
        if !options_done {
            if let Some(target) = arg.strip_prefix("--target-directory=") {
                if !target.is_empty() {
                    target_directory = Some(target);
                }
                i += 1;
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                i += 1;
                continue;
            }
        }
        operands.push(arg);
        i += 1;
    }
    target_directory.or_else(|| operands.last().copied())
}

fn chezmoi_is_apply_or_ambiguous(args: &[String]) -> bool {
    match chezmoi_subcommand(args) {
        Ok(Some(command)) => command == "apply",
        Ok(None) => false,
        // A malformed known global option must fail closed under no_shell_mutation.
        Err(()) => true,
    }
}

fn chezmoi_subcommand(args: &[String]) -> Result<Option<&str>, ()> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            return Ok(args.get(i + 1).map(String::as_str));
        }
        if matches!(
            arg,
            "--source" | "-S" | "--destination" | "-D" | "--config" | "--cache"
        ) {
            if args.get(i + 1).is_none() {
                return Err(());
            }
            i += 2;
            continue;
        }
        if arg.starts_with("--source=")
            || arg.starts_with("--destination=")
            || arg.starts_with("--config=")
            || arg.starts_with("--cache=")
        {
            if arg.ends_with('=') {
                return Err(());
            }
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        return Ok(Some(arg));
    }
    Ok(None)
}

fn looks_like_dotfile(path: &str) -> bool {
    // `.zshrc`, `.config/...`, or any `.../.hidden` segment.
    path.starts_with('.') || path.contains("/.")
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

    #[test]
    fn shell_mutation_ln_symlink() {
        let args: Vec<String> = ["-s", "/tmp/a", ".zshrc"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(classify_child("ln", &args).contains(&Intent::ShellMutation));

        let args_sf: Vec<String> = ["-sf", "a", "b"].into_iter().map(str::to_string).collect();
        assert!(classify_child("ln", &args_sf).contains(&Intent::ShellMutation));
    }

    #[test]
    fn shell_mutation_non_mutating_lookalike() {
        // `ln` without `-s` must not fire.
        let args: Vec<String> = ["a", "b"].into_iter().map(str::to_string).collect();
        assert!(!classify_child("ln", &args).contains(&Intent::ShellMutation));

        let ls: Vec<String> = ["-la"].into_iter().map(str::to_string).collect();
        assert!(!classify_child("ls", &ls).contains(&Intent::ShellMutation));
    }

    #[test]
    fn shell_mutation_chezmoi_apply() {
        let args: Vec<String> = ["apply"].into_iter().map(str::to_string).collect();
        assert!(classify_child("chezmoi", &args).contains(&Intent::ShellMutation));
    }

    #[test]
    fn shell_mutation_chezmoi_option_prefixed_apply() {
        for args in [
            vec!["--source", "/tmp/src", "apply"],
            vec!["--source=/tmp/src", "apply"],
            vec!["-D", "/tmp/dest", "apply"],
            vec!["--", "apply"],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            assert!(
                classify_child("chezmoi", &args).contains(&Intent::ShellMutation),
                "args={args:?}"
            );
        }
    }

    #[test]
    fn shell_mutation_chezmoi_read_only_and_malformed_options() {
        let read_only: Vec<String> = ["--source", "/tmp/src", "status"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(!classify_child("chezmoi", &read_only).contains(&Intent::ShellMutation));

        let malformed: Vec<String> = ["--source"].into_iter().map(str::to_string).collect();
        assert!(classify_child("chezmoi", &malformed).contains(&Intent::ShellMutation));
    }

    #[test]
    fn shell_mutation_cp_dotfile_dest() {
        let args: Vec<String> = ["src", ".bashrc"].into_iter().map(str::to_string).collect();
        assert!(classify_child("cp", &args).contains(&Intent::ShellMutation));

        let nested: Vec<String> = ["src", ".config/git/config"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(classify_child("cp", &nested).contains(&Intent::ShellMutation));

        let plain: Vec<String> = ["src", "README.md"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(!classify_child("cp", &plain).contains(&Intent::ShellMutation));
    }

    #[test]
    fn shell_mutation_transfer_target_directory() {
        for args in [
            vec!["-t", ".config", "source"],
            vec!["--target-directory=.config", "source"],
            vec!["--target-directory", ".config", "source"],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            assert!(
                classify_child("cp", &args).contains(&Intent::ShellMutation),
                "args={args:?}"
            );
        }
    }

    #[test]
    fn bin_mutation_still_fires() {
        let args: Vec<String> = ["bin/foo"].into_iter().map(str::to_string).collect();
        assert!(classify_child("rm", &args).contains(&Intent::BinMutation));

        let mv: Vec<String> = ["lib/a", "lib/b"].into_iter().map(str::to_string).collect();
        assert!(classify_child("mv", &mv).contains(&Intent::BinMutation));

        let archive: Vec<String> = ["bin-cleanup", "--mode", "archive"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(classify_child("ontarch", &archive).contains(&Intent::BinMutation));
    }
}

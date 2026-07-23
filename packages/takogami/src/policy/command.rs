//! Restricted command-pattern parser and token-prefix matcher.

use sha2::{Digest, Sha256};

use super::raw::{PolicyContractKind, RawPolicyError};

const FORBIDDEN: &[&str] = &[
    "|", "||", "&", "&&", ";", ">", ">>", "<", "<<", "$(", "`", "${", "\n", "\r",
];

/// Parse a policy/profile command pattern into tokens (no deprecation warning).
pub fn parse_command_pattern(input: &str, origin_id: &str) -> Result<Vec<String>, RawPolicyError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            "empty command pattern",
            Some(origin_id.into()),
            Some("commands".into()),
        ));
    }
    for token in FORBIDDEN {
        if trimmed.contains(token) {
            return Err(RawPolicyError::new(
                PolicyContractKind::PolicyRuleInvalid,
                format!("shell syntax rejected in command pattern: `{token}`"),
                Some(origin_id.into()),
                Some("commands".into()),
            ));
        }
    }
    if trimmed.contains('$') {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            "shell syntax rejected in command pattern: `$`",
            Some(origin_id.into()),
            Some("commands".into()),
        ));
    }
    let tokens = split_argv(trimmed, origin_id)?;
    if tokens.is_empty() || tokens[0].is_empty() {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            "command pattern has empty program token",
            Some(origin_id.into()),
            Some("commands".into()),
        ));
    }
    if tokens[0].contains("..") {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            "command pattern program must not contain path traversal",
            Some(origin_id.into()),
            Some("commands".into()),
        ));
    }
    Ok(tokens)
}

fn split_argv(input: &str, origin_id: &str) -> Result<Vec<String>, RawPolicyError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            '\\' if in_quotes => match chars.next() {
                Some(escaped) => cur.push(escaped),
                None => {
                    return Err(RawPolicyError::new(
                        PolicyContractKind::PolicyRuleInvalid,
                        "malformed quoting: trailing backslash",
                        Some(origin_id.into()),
                        Some("commands".into()),
                    ));
                }
            },
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if in_quotes {
        return Err(RawPolicyError::new(
            PolicyContractKind::PolicyRuleInvalid,
            "malformed quoting: unmatched `\"`",
            Some(origin_id.into()),
            Some("commands".into()),
        ));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// Token-prefix match: pattern tokens must equal the leading argv tokens exactly.
pub fn matches_token_prefix(pattern: &[String], argv: &[String]) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if argv.len() < pattern.len() {
        return false;
    }
    pattern.iter().zip(argv.iter()).all(|(p, a)| p == a)
}

/// Compare program identity: exact token, or basename of an absolute program path.
pub fn program_matches(pattern_program: &str, actual_program: &str) -> bool {
    if pattern_program == actual_program {
        return true;
    }
    let actual_base = std::path::Path::new(actual_program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(actual_program);
    pattern_program == actual_base
}

/// Match a full command pattern against program + argv (argv excludes program).
pub fn matches_command(pattern: &[String], program: &str, args: &[String]) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if !program_matches(&pattern[0], program) {
        return false;
    }
    matches_token_prefix(&pattern[1..], args)
}

/// Short digest of canonical matcher payload for stable rule IDs.
pub fn matcher_digest(payload: &str) -> String {
    let digest = Sha256::digest(payload.as_bytes());
    format!("{digest:x}")[..12].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_prefix_respects_boundaries() {
        let pattern = parse_command_pattern("git push", "t").unwrap();
        assert!(matches_command(
            &pattern,
            "git",
            &["push".into(), "origin".into()]
        ));
        assert!(!matches_command(&pattern, "git", &["pushy".into()]));
        let rm = parse_command_pattern("rm -rf", "t").unwrap();
        assert!(!matches_command(&rm, "rm", &["-r".into(), "-f".into()]));
    }

    #[test]
    fn rejects_shell_syntax() {
        assert!(parse_command_pattern("echo hi | cat", "t").is_err());
        assert!(parse_command_pattern("echo $HOME", "t").is_err());
    }
}

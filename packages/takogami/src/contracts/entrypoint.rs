//! Constrained legacy string-entrypoint parser.
//!
//! Accepts simple argv only. Rejects shell operators, substitutions, redirections,
//! and malformed quoting. Successful parses carry a deprecation diagnostic.

use crate::contracts::types::DiagnosticRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyEntrypoint {
    pub program: String,
    pub args: Vec<String>,
    pub deprecation: DiagnosticRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyParseError {
    pub message: String,
}

impl LegacyParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

const FORBIDDEN: &[&str] = &[
    "|", "||", "&", "&&", ";", ">", ">>", "<", "<<", "$(", "`", "${", "\n", "\r",
];

/// Parse a legacy entrypoint string into program + argv.
///
/// Whitespace separates tokens. Double-quoted segments keep internal spaces literal.
pub fn parse_legacy_entrypoint(input: &str) -> Result<LegacyEntrypoint, LegacyParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(LegacyParseError::new("empty entrypoint"));
    }
    for token in FORBIDDEN {
        if trimmed.contains(token) {
            return Err(LegacyParseError::new(format!(
                "shell syntax rejected: contains `{token}`"
            )));
        }
    }
    if trimmed.contains('$') {
        return Err(LegacyParseError::new(
            "shell syntax rejected: contains `$` substitution",
        ));
    }

    let argv = split_argv(trimmed)?;
    let Some((program, rest)) = argv.split_first() else {
        return Err(LegacyParseError::new("empty entrypoint"));
    };
    if program.is_empty() {
        return Err(LegacyParseError::new("empty program"));
    }

    Ok(LegacyEntrypoint {
        program: program.clone(),
        args: rest.to_vec(),
        deprecation: DiagnosticRecord {
            code: "legacy_entrypoint_deprecated".to_string(),
            message: "string entrypoints are deprecated; use structured program/args".to_string(),
        },
    })
}

fn split_argv(input: &str) -> Result<Vec<String>, LegacyParseError> {
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
                    return Err(LegacyParseError::new(
                        "malformed quoting: trailing backslash",
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
        return Err(LegacyParseError::new("malformed quoting: unmatched `\"`"));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_argv() {
        let parsed = parse_legacy_entrypoint("moon run takogami:build").unwrap();
        assert_eq!(parsed.program, "moon");
        assert_eq!(parsed.args, vec!["run", "takogami:build"]);
        assert_eq!(parsed.deprecation.code, "legacy_entrypoint_deprecated");
    }

    #[test]
    fn keeps_quoted_whitespace_literal() {
        let parsed = parse_legacy_entrypoint(r#"echo "hello world""#).unwrap();
        assert_eq!(parsed.program, "echo");
        assert_eq!(parsed.args, vec!["hello world"]);
    }

    #[test]
    fn rejects_pipe() {
        let err = parse_legacy_entrypoint("cargo build | less").unwrap_err();
        assert!(err.message.contains("|"));
    }

    #[test]
    fn rejects_substitution() {
        let err = parse_legacy_entrypoint("echo $HOME").unwrap_err();
        assert!(err.message.contains("$"));
    }

    #[test]
    fn rejects_redirection() {
        assert!(parse_legacy_entrypoint("cargo build > out").is_err());
    }

    #[test]
    fn rejects_unmatched_quote() {
        assert!(parse_legacy_entrypoint(r#"echo "hi"#).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_legacy_entrypoint("   ").is_err());
    }
}

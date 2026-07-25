//! Optional RTK postprocessing — never replaces the authorized child.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::contracts::types::DiagnosticRecord;
use crate::execution::{StreamCapture, StreamEncoding};

#[derive(Debug, Clone, PartialEq)]
pub struct RtkResult {
    pub compressor: String,
    pub gain: Option<f64>,
    pub emitted: Vec<u8>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

/// Exact filter eligibility from the authorized child form (program basename + first arg).
pub fn rtk_filter_for(program: &str, argv: &[String]) -> Option<&'static str> {
    let base = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    match base {
        "git" => match argv.first().map(String::as_str) {
            Some("status") => Some("git-status"),
            Some("diff") => Some("git-diff"),
            Some("log") => Some("git-log"),
            _ => None,
        },
        "grep" => Some("grep"),
        "rg" => Some("rg"),
        _ => None,
    }
}

pub fn resolve_rtk_binary(path_dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in path_dirs {
        let candidate = dir.join("rtk");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn probe_rtk(rtk: &Path) -> bool {
    let gain = Command::new(rtk)
        .args(["gain", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let pipe = Command::new(rtk)
        .args(["pipe", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    gain && pipe
}

/// Postprocess one bounded UTF-8 stream. On any failure, return original bytes exactly.
pub fn pipe_stream(rtk: &Path, filter: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = Command::new(rtk)
        .args(["pipe", "--filter", filter])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("RTK_TELEMETRY", "0")
        .spawn()
        .map_err(|e| e.to_string())?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "rtk stdin missing".to_string())?;
        stdin.write_all(input).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("rtk pipe exit {}", output.status));
    }
    Ok(output.stdout)
}

pub fn apply_rtk_if_eligible(
    enabled: bool,
    json_mode: bool,
    profile_is_dev: bool,
    program: &str,
    argv: &[String],
    capture: &StreamCapture,
    path_dirs: &[PathBuf],
) -> RtkResult {
    if json_mode || !enabled {
        return RtkResult {
            compressor: "disabled".into(),
            gain: None,
            emitted: capture.bytes.clone(),
            diagnostics: vec![],
        };
    }
    if profile_is_dev {
        return raw("unsupported", capture, None);
    }
    let Some(filter) = rtk_filter_for(program, argv) else {
        return raw("unsupported", capture, None);
    };
    if capture.truncated || capture.encoding != StreamEncoding::Utf8 {
        return raw("unsupported", capture, None);
    }
    let Some(rtk) = resolve_rtk_binary(path_dirs) else {
        return raw(
            "unavailable",
            capture,
            Some(("rtk_unavailable", "rtk binary not found")),
        );
    };
    if !probe_rtk(&rtk) {
        return raw(
            "unavailable",
            capture,
            Some(("rtk_unavailable", "rtk capability probe failed")),
        );
    }
    match pipe_stream(&rtk, filter, &capture.bytes) {
        Ok(out) if out.len() < capture.bytes.len() => {
            let raw_len = capture.bytes.len() as f64;
            let gain = if raw_len > 0.0 {
                Some((raw_len - out.len() as f64) / raw_len)
            } else {
                None
            };
            RtkResult {
                compressor: "rtk".into(),
                gain,
                emitted: out,
                diagnostics: vec![],
            }
        }
        Ok(_) => raw("unsupported", capture, None),
        Err(msg) => raw("failed", capture, Some(("rtk_failed", &truncate(&msg)))),
    }
}

fn raw(compressor: &str, capture: &StreamCapture, diag: Option<(&str, &str)>) -> RtkResult {
    RtkResult {
        compressor: compressor.into(),
        gain: None,
        emitted: capture.bytes.clone(),
        diagnostics: diag
            .map(|(code, message)| {
                vec![DiagnosticRecord {
                    code: code.into(),
                    message: message.into(),
                }]
            })
            .unwrap_or_default(),
    }
}

fn truncate(msg: &str) -> String {
    const MAX: usize = 4096;
    if msg.len() > MAX {
        msg[..MAX].to_string()
    } else {
        msg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_mapping_is_exact() {
        assert_eq!(
            rtk_filter_for("git", &["status".into()]),
            Some("git-status")
        );
        assert_eq!(rtk_filter_for("rg", &["foo".into()]), Some("rg"));
        assert_eq!(rtk_filter_for("moon", &["run".into()]), None);
    }

    #[test]
    fn json_never_invokes_mapping_side_effects() {
        let capture = StreamCapture {
            bytes: b"hello".to_vec(),
            total_bytes: 5,
            truncated: false,
            encoding: StreamEncoding::Utf8,
            broken_pipe: false,
            read_error: None,
        };
        let result =
            apply_rtk_if_eligible(true, true, false, "git", &["status".into()], &capture, &[]);
        assert_eq!(result.compressor, "disabled");
        assert_eq!(result.emitted, b"hello");
    }
}

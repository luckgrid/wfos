//! Optional RTK postprocessing — never replaces the authorized child.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::contracts::types::DiagnosticRecord;
use crate::execution::{StreamCapture, StreamEncoding};

/// Short bound on one adapter invocation (S6.1-06). RTK only postprocesses already-captured,
/// bounded child output, so a healthy adapter finishes well under this.
const RTK_PIPE_TIMEOUT: Duration = Duration::from_millis(1500);

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

/// Accept only a canonical, regular, executable file — never a directory, device, or file
/// lacking the execute bit (S6.1-06 identity requirement).
fn canonical_rtk_identity(candidate: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(candidate).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return None;
    }
    candidate.canonicalize().ok()
}

/// Resolve the RTK adapter's canonical identity from the documented fallback resolver (first
/// `rtk` found on the supplied search path). Canonicalized once so later revalidation can
/// detect replacement/drift by path-identity comparison.
pub fn resolve_rtk_binary(path_dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in path_dirs {
        let candidate = dir.join("rtk");
        if let Some(canonical) = canonical_rtk_identity(&candidate) {
            return Some(canonical);
        }
    }
    None
}

fn adapter_command(rtk: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(rtk);
    cmd.args(args).env_clear().env("RTK_TELEMETRY", "0");
    // New process group so a timeout/cleanup can target descendants via kill(-pgid) without
    // touching the authorized child's own group.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd
}

pub fn probe_rtk(rtk: &Path) -> bool {
    let gain = adapter_command(rtk, &["gain", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let pipe = adapter_command(rtk, &["pipe", "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    gain && pipe
}

struct BoundedRead {
    bytes: Vec<u8>,
    truncated: bool,
}

/// Drain a stream on its own thread into an independently-bounded buffer. Reading continues
/// past the cap (discarding further bytes) so the adapter is never blocked on a full pipe
/// while the caller is deciding how to handle an oversized stream.
fn read_bounded(mut reader: impl Read, cap: usize) -> BoundedRead {
    let mut buf = [0u8; 8192];
    let mut bytes = Vec::new();
    let mut truncated = false;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(bytes.len());
                if room > 0 {
                    bytes.extend_from_slice(&buf[..room.min(n)]);
                }
                if n > room {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    BoundedRead { bytes, truncated }
}

/// Postprocess one bounded UTF-8 stream. On any failure (including timeout, oversized output,
/// or a non-success exit), return an error so the caller falls back to the original bytes
/// exactly (S6.1-06). Stdin is written and stdout/stderr are drained concurrently on separate
/// threads to avoid the classic pipe deadlock when the adapter echoes more than one pipe
/// buffer's worth of data. On timeout the adapter's whole process group is killed and reaped
/// before returning.
pub fn pipe_stream(rtk: &Path, filter: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = adapter_command(rtk, &["pipe", "--filter", filter])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id() as libc::pid_t;

    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("rtk stdin missing".to_string());
        }
    };
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("rtk stdout missing".to_string());
        }
    };
    let mut stderr = match child.stderr.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("rtk stderr missing".to_string());
        }
    };

    // Cap each stream independently, generously relative to the input: a healthy filter never
    // needs to emit many multiples of what it was given.
    let out_cap = input.len().saturating_mul(4).max(64 * 1024);
    let input_owned = input.to_vec();
    let writer = thread::spawn(move || {
        let result = stdin.write_all(&input_owned);
        drop(stdin); // Close the write end so the adapter observes EOF on stdin.
        result
    });
    let stdout_reader = thread::spawn(move || read_bounded(&mut stdout, out_cap));
    let stderr_reader = thread::spawn(move || read_bounded(&mut stderr, out_cap));

    let deadline = Instant::now() + RTK_PIPE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    break Err("rtk pipe timed out".to_string());
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => break Err(e.to_string()),
        }
    };

    let status = match status {
        Ok(status) => status,
        Err(reason) => {
            // Timeout or wait error: terminate the adapter's process group and reap before
            // returning, so no hung adapter lingers past this call.
            unsafe {
                let _ = libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.wait();
            let _ = writer.join();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(reason);
        }
    };

    let _ = writer.join();
    let stdout_result = stdout_reader
        .join()
        .map_err(|_| "rtk stdout reader thread panicked".to_string())?;
    let _stderr_result = stderr_reader
        .join()
        .map_err(|_| "rtk stderr reader thread panicked".to_string())?;
    // Stderr is intentionally discarded here: it must never reach user child output or durable
    // records (S6.1-06).

    if stdout_result.truncated {
        return Err("rtk pipe output exceeded bound".to_string());
    }
    if !status.success() {
        return Err(format!("rtk pipe exit {status}"));
    }
    Ok(stdout_result.bytes)
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
    // Revalidate canonical identity immediately before spawning the adapter, closing the
    // window between the probe and the pipe invocation in which the binary could have been
    // replaced (S6.1-06). Any drift is treated as unavailable, never as a reason to spawn an
    // unverified binary.
    match resolve_rtk_binary(path_dirs) {
        Some(revalidated) if revalidated == rtk => {}
        _ => {
            return raw(
                "unavailable",
                capture,
                Some((
                    "rtk_identity_drift",
                    "rtk binary identity changed between probe and pipe",
                )),
            );
        }
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

    fn write_script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn capture_of(bytes: &[u8]) -> StreamCapture {
        StreamCapture {
            bytes: bytes.to_vec(),
            total_bytes: bytes.len() as u64,
            truncated: false,
            encoding: StreamEncoding::Utf8,
            broken_pipe: false,
            read_error: None,
        }
    }

    const PROBE_OK: &str = r#"case "$1 $2" in
  "gain --help") exit 0 ;;
  "pipe --help") exit 0 ;;
esac"#;

    // S6.1-06: sequential write-then-drain deadlocks once the adapter echoes more than one
    // pipe buffer's worth of data. Concurrent stdin write / stdout drain must not.
    #[test]
    fn pipe_stream_drains_concurrently_avoiding_deadlock_on_large_echo() {
        let dir = tempfile::tempdir().unwrap();
        let rtk = dir.path().join("rtk");
        write_script(&rtk, &format!("{PROBE_OK}\nexec cat\n"));

        let input = vec![b'a'; 500_000];
        let started = Instant::now();
        let result = pipe_stream(&rtk, "rg", &input);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "concurrent drain must not deadlock on a large echo"
        );
        assert_eq!(result, Ok(input));
    }

    // S6.1-06: an adapter that multiplies output far beyond its input must be rejected, not
    // accepted with partial content.
    #[test]
    fn pipe_stream_rejects_output_exceeding_bound() {
        let dir = tempfile::tempdir().unwrap();
        let rtk = dir.path().join("rtk");
        write_script(
            &rtk,
            &format!(
                "{PROBE_OK}\ndata=$(cat)\ni=0\nwhile [ \"$i\" -lt 6 ]; do printf '%s' \"$data\"; i=$((i+1)); done\n"
            ),
        );

        let input = vec![b'x'; 20_000];
        let result = pipe_stream(&rtk, "rg", &input);
        assert_eq!(result, Err("rtk pipe output exceeded bound".to_string()));
    }

    // S6.1-06: a hung adapter must be killed and reaped within the short bound, never left
    // running or blocking the caller indefinitely.
    #[test]
    fn pipe_stream_times_out_and_kills_hung_adapter() {
        let dir = tempfile::tempdir().unwrap();
        let rtk = dir.path().join("rtk");
        write_script(&rtk, &format!("{PROBE_OK}\nsleep 30\n"));

        let started = Instant::now();
        let result = pipe_stream(&rtk, "rg", b"hi");
        let elapsed = started.elapsed();
        assert_eq!(result, Err("rtk pipe timed out".to_string()));
        assert!(
            elapsed < Duration::from_secs(3),
            "timeout must bound the wait, took {elapsed:?}"
        );
    }

    #[test]
    fn pipe_stream_rejects_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let rtk = dir.path().join("rtk");
        write_script(&rtk, &format!("{PROBE_OK}\ncat >/dev/null\nexit 7\n"));

        let result = pipe_stream(&rtk, "rg", b"hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("rtk pipe exit"));
    }

    // S6.1-06 end-to-end: a well-behaved adapter that returns strictly smaller valid output is
    // accepted through the full resolve -> probe -> revalidate -> pipe pipeline.
    #[test]
    fn apply_rtk_if_eligible_accepts_valid_smaller_result() {
        let dir = tempfile::tempdir().unwrap();
        let rtk = dir.path().join("rtk");
        write_script(&rtk, &format!("{PROBE_OK}\ncat | head -c 5\n"));

        let capture = capture_of(b"hello world");
        let result = apply_rtk_if_eligible(
            true,
            false,
            false,
            "git",
            &["status".into()],
            &capture,
            &[dir.path().to_path_buf()],
        );
        assert_eq!(result.compressor, "rtk");
        assert_eq!(result.emitted, b"hello");
        assert!(result.gain.unwrap() > 0.0);
    }

    // S6.1-06: if the on-disk identity changes between the probe and the pipe invocation, the
    // pipe must never be spawned against the replaced binary; fall back to raw as unavailable.
    #[test]
    fn apply_rtk_if_eligible_rejects_identity_drift_between_probe_and_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path();
        let real_a = dir_path.join("real_a");
        let real_b = dir_path.join("real_b");
        let symlink = dir_path.join("rtk");

        write_script(
            &real_a,
            &format!(
                "case \"$1 $2\" in\n  \"gain --help\") ln -sf {b} {link}; exit 0 ;;\n  \"pipe --help\") exit 0 ;;\nesac\nexit 1",
                b = real_b.display(),
                link = symlink.display(),
            ),
        );
        write_script(
            &real_b,
            "case \"$1 $2\" in\n  \"gain --help\") exit 0 ;;\n  \"pipe --help\") exit 0 ;;\nesac\nexit 1",
        );
        std::os::unix::fs::symlink(&real_a, &symlink).unwrap();

        let capture = capture_of(b"hello world");
        let result = apply_rtk_if_eligible(
            true,
            false,
            false,
            "git",
            &["status".into()],
            &capture,
            &[dir_path.to_path_buf()],
        );
        assert_eq!(result.compressor, "unavailable");
        assert_eq!(result.emitted, b"hello world");
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "rtk_identity_drift");
    }
}

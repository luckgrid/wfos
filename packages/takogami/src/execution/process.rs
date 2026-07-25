//! Tokio-backed exact spawn of an AuthorizedExecutionPlan (no shell).

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use super::environment::{EnvError, snapshot_env};
use super::signals::{ProcessGroupGuard, SignalSource, UnixSignalSource, signal_name};
use super::streams::{StreamCapture, StreamEncoding, capture_pipe, stream_or_buffer};
use super::{ExecutionMode, ExecutionOptions, ExecutionReport, Executor, StreamSummary};
use crate::contracts::types::DiagnosticRecord;
use crate::exit_codes;
use crate::output::apply_rtk_if_eligible;
use crate::policy::AuthorizedExecutionPlan;

#[derive(Debug, Default, Clone, Copy)]
pub struct TokioExecutor;

#[async_trait]
impl Executor for TokioExecutor {
    async fn execute(
        &self,
        plan: &AuthorizedExecutionPlan,
        options: &ExecutionOptions,
    ) -> ExecutionReport {
        match execute_inner(plan, options).await {
            Ok(report) => report,
            Err(err) => err.into_report(),
        }
    }
}

struct ExecFailure {
    outcome: String,
    diagnostics: Vec<DiagnosticRecord>,
    spawned: bool,
    pid: Option<u32>,
}

impl ExecFailure {
    fn contract(message: impl Into<String>) -> Self {
        Self {
            outcome: "failed_to_spawn".into(),
            diagnostics: vec![DiagnosticRecord {
                code: "execution_contract".into(),
                message: message.into(),
            }],
            spawned: false,
            pid: None,
        }
    }

    fn io(code: &str, message: impl Into<String>) -> Self {
        Self {
            outcome: "failed_to_spawn".into(),
            diagnostics: vec![DiagnosticRecord {
                code: code.into(),
                message: message.into(),
            }],
            spawned: false,
            pid: None,
        }
    }

    fn into_report(self) -> ExecutionReport {
        let mut report = ExecutionReport::idle(self.outcome);
        report.spawned = self.spawned;
        report.pid = self.pid;
        report.diagnostics = self.diagnostics;
        report.compressor = "none".into();
        report
    }
}

async fn execute_inner(
    authorized: &AuthorizedExecutionPlan,
    options: &ExecutionOptions,
) -> Result<ExecutionReport, ExecFailure> {
    let sealed = authorized.plan();
    let exe = sealed.executable_path();
    let cwd = sealed.cwd_path();
    let resolved = sealed.resolved();

    preflight_identity(exe, cwd)?;

    let env = snapshot_env(&resolved.env_keys).map_err(|e: EnvError| {
        let d = e.diagnostic();
        ExecFailure {
            outcome: "failed_to_spawn".into(),
            diagnostics: vec![d],
            spawned: false,
            pid: None,
        }
    })?;

    let limit = options.limits.capture_limit;
    let mut cmd = Command::new(exe);
    cmd.args(&resolved.argv)
        .current_dir(cwd)
        .env_clear()
        .envs(env.pairs.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // New process group so signals can target descendants via kill(-pgid).
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|e| {
        ExecFailure::io(
            "execution_spawn",
            format!("failed to spawn authorized child: {e}"),
        )
    })?;

    let pid = child.id();
    let mut guard = pid.map(ProcessGroupGuard::new);
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ExecFailure::io("execution_io", "child stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ExecFailure::io("execution_io", "child stderr pipe missing"))?;

    let mut signals = UnixSignalSource::install().map_err(|e| {
        ExecFailure::io(
            "execution_signal",
            format!("failed to install signal handlers: {e}"),
        )
    })?;

    let (stdout_cap, stderr_cap, status) = match &options.mode {
        ExecutionMode::Json => {
            run_capturing(
                &mut child,
                &mut signals,
                guard.as_mut(),
                capture_pipe(stdout, limit),
                capture_pipe(stderr, limit),
            )
            .await
        }
        ExecutionMode::Human {
            rtk_eligible,
            profile_id: _,
        } => {
            let buffer_for_rtk = *rtk_eligible;
            let flush_at_eof = !buffer_for_rtk;
            run_capturing(
                &mut child,
                &mut signals,
                guard.as_mut(),
                stream_or_buffer(
                    stdout,
                    super::streams::StreamDest::Stdout,
                    limit,
                    buffer_for_rtk,
                    flush_at_eof,
                ),
                stream_or_buffer(
                    stderr,
                    super::streams::StreamDest::Stderr,
                    limit,
                    buffer_for_rtk,
                    flush_at_eof,
                ),
            )
            .await
        }
    };

    if let Some(g) = guard.as_mut() {
        g.disarm();
    }

    let status = status.map_err(|e| {
        let mut f = ExecFailure::io("execution_io", format!("failed to wait for child: {e}"));
        f.spawned = true;
        f.pid = pid;
        f
    })?;

    let (exit_code, signal, outcome) = map_status(&status);

    let mut diagnostics = env.diagnostics;
    push_stream_diags(&mut diagnostics, "stdout", &stdout_cap);
    push_stream_diags(&mut diagnostics, "stderr", &stderr_cap);

    let (compressor, gain, emitted_output_bytes, stdout_summary, stderr_summary) = finalize_output(
        authorized,
        options,
        &stdout_cap,
        &stderr_cap,
        &mut diagnostics,
    );

    Ok(ExecutionReport {
        spawned: true,
        pid,
        exit_code,
        signal,
        outcome,
        stdout: stdout_summary,
        stderr: stderr_summary,
        diagnostics,
        compressor,
        gain,
        emitted_output_bytes,
    })
}

async fn run_capturing(
    child: &mut tokio::process::Child,
    signals: &mut dyn SignalSource,
    mut guard: Option<&mut ProcessGroupGuard>,
    out: impl std::future::Future<Output = StreamCapture>,
    err: impl std::future::Future<Output = StreamCapture>,
) -> (
    StreamCapture,
    StreamCapture,
    Result<std::process::ExitStatus, io::Error>,
) {
    tokio::pin!(out);
    tokio::pin!(err);

    let mut out_done: Option<StreamCapture> = None;
    let mut err_done: Option<StreamCapture> = None;
    let mut status_done: Option<Result<std::process::ExitStatus, io::Error>> = None;
    let mut forwarded = false;

    loop {
        if out_done.is_some() && err_done.is_some() && status_done.is_some() {
            break;
        }
        tokio::select! {
            biased;
            capt = &mut out, if out_done.is_none() => {
                out_done = Some(capt);
            }
            capt = &mut err, if err_done.is_none() => {
                err_done = Some(capt);
            }
            st = child.wait(), if status_done.is_none() => {
                status_done = Some(st);
            }
            maybe_sig = signals.recv() => {
                if let Some(sig) = maybe_sig
                    && let Some(g) = guard.as_mut()
                {
                    if !forwarded {
                        g.signal_group(sig);
                        forwarded = true;
                    } else {
                        g.signal_group(libc::SIGKILL);
                    }
                }
            }
        }
    }

    (
        out_done.unwrap_or_else(StreamCapture::empty),
        err_done.unwrap_or_else(StreamCapture::empty),
        status_done.unwrap_or_else(|| Err(io::Error::other("child wait missing"))),
    )
}

fn preflight_identity(exe: &Path, cwd: &Path) -> Result<(), ExecFailure> {
    let exe_now = accept_regular_executable(exe).ok_or_else(|| {
        ExecFailure::contract("sealed executable is no longer a regular executable file")
    })?;
    if exe_now.as_path() != exe {
        return Err(ExecFailure::contract(
            "sealed executable canonical identity drifted (execution_contract_changed)",
        ));
    }

    let cwd_meta = fs::metadata(cwd)
        .map_err(|_| ExecFailure::contract("sealed cwd is no longer accessible"))?;
    if !cwd_meta.is_dir() {
        return Err(ExecFailure::contract("sealed cwd is no longer a directory"));
    }
    let cwd_now = cwd
        .canonicalize()
        .map_err(|_| ExecFailure::contract("sealed cwd canonicalization failed"))?;
    if cwd_now.as_path() != cwd {
        return Err(ExecFailure::contract(
            "sealed cwd canonical identity drifted (execution_contract_changed)",
        ));
    }
    Ok(())
}

fn accept_regular_executable(path: &Path) -> Option<PathBuf> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.permissions().mode() & 0o111 == 0 {
        return None;
    }
    path.canonicalize().ok()
}

fn map_status(status: &std::process::ExitStatus) -> (Option<u8>, Option<String>, String) {
    if let Some(sig) = status.signal() {
        let name = signal_name(sig).to_string();
        let code = exit_codes::exit_from_signal_number(sig);
        return (Some(code), Some(name), "interrupted".into());
    }
    let code = status.code().map(|c| c.clamp(0, 255) as u8).unwrap_or(1);
    (Some(code), None, "completed".into())
}

fn push_stream_diags(diags: &mut Vec<DiagnosticRecord>, label: &str, cap: &StreamCapture) {
    if let Some(msg) = &cap.read_error {
        diags.push(DiagnosticRecord {
            code: "execution_stream".into(),
            message: format!("{label}: {msg}"),
        });
    }
    if cap.broken_pipe {
        diags.push(DiagnosticRecord {
            code: "broken_pipe".into(),
            message: format!("{label}: output consumer closed (broken pipe)"),
        });
    }
}

fn finalize_output(
    authorized: &AuthorizedExecutionPlan,
    options: &ExecutionOptions,
    stdout_cap: &StreamCapture,
    stderr_cap: &StreamCapture,
    diagnostics: &mut Vec<DiagnosticRecord>,
) -> (String, Option<f64>, u64, StreamSummary, StreamSummary) {
    match &options.mode {
        ExecutionMode::Json => {
            let emitted = stdout_cap.bytes.len() as u64 + stderr_cap.bytes.len() as u64;
            (
                "none".into(),
                None,
                emitted,
                StreamSummary::from_capture(stdout_cap),
                StreamSummary::from_capture(stderr_cap),
            )
        }
        ExecutionMode::Human {
            rtk_eligible,
            profile_id: _,
        } => {
            let resolved = authorized.plan().resolved();
            let is_dev = authorized.request().verb == "dev";
            let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default();

            if *rtk_eligible
                && !stdout_cap.truncated
                && !stderr_cap.truncated
                && stdout_cap.encoding == StreamEncoding::Utf8
                && stderr_cap.encoding == StreamEncoding::Utf8
            {
                let out_rtk = apply_rtk_if_eligible(
                    true,
                    false,
                    is_dev,
                    &resolved.program,
                    &resolved.argv,
                    stdout_cap,
                    &path_dirs,
                );
                let err_rtk = apply_rtk_if_eligible(
                    true,
                    false,
                    is_dev,
                    &resolved.program,
                    &resolved.argv,
                    stderr_cap,
                    &path_dirs,
                );
                diagnostics.extend(out_rtk.diagnostics.iter().cloned());
                diagnostics.extend(err_rtk.diagnostics.iter().cloned());

                let mut broken = false;
                {
                    let mut out = io::stdout().lock();
                    let _ = write_ignore_pipe(&mut out, &out_rtk.emitted, &mut broken);
                }
                {
                    let mut err = io::stderr().lock();
                    let _ = write_ignore_pipe(&mut err, &err_rtk.emitted, &mut broken);
                }
                if broken {
                    diagnostics.push(DiagnosticRecord {
                        code: "broken_pipe".into(),
                        message: "output consumer closed while emitting human bytes".into(),
                    });
                }

                let compressor = if out_rtk.compressor == "rtk" || err_rtk.compressor == "rtk" {
                    "rtk".into()
                } else if out_rtk.compressor != "none" {
                    out_rtk.compressor.clone()
                } else {
                    err_rtk.compressor.clone()
                };
                let gain = out_rtk.gain.or(err_rtk.gain);
                let emitted = out_rtk.emitted.len() as u64 + err_rtk.emitted.len() as u64;
                (
                    compressor,
                    gain,
                    emitted,
                    StreamSummary::from_capture(stdout_cap),
                    StreamSummary::from_capture(stderr_cap),
                )
            } else {
                let emitted = stdout_cap
                    .total_bytes
                    .saturating_add(stderr_cap.total_bytes);
                (
                    if *rtk_eligible && (stdout_cap.truncated || stderr_cap.truncated) {
                        "unsupported".into()
                    } else {
                        "none".into()
                    },
                    None,
                    emitted,
                    StreamSummary::from_capture(stdout_cap),
                    StreamSummary::from_capture(stderr_cap),
                )
            }
        }
    }
}

fn write_ignore_pipe(writer: &mut dyn Write, bytes: &[u8], broken: &mut bool) -> io::Result<()> {
    if *broken || bytes.is_empty() {
        return Ok(());
    }
    match writer.write_all(bytes) {
        Ok(()) => {
            let _ = writer.flush();
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
            *broken = true;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

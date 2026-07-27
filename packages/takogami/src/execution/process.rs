//! Tokio-backed exact spawn of an AuthorizedExecutionPlan (no shell).

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use super::environment::{EnvError, build_child_env, snapshot_env};
use super::signals::{ProcessGroupGuard, SignalSource, UnixSignalSource, signal_name};
use super::streams::{StreamCapture, StreamEncoding, capture_pipe, stream_or_buffer};
use super::{ExecutionMode, ExecutionOptions, ExecutionReport, Executor, StreamSummary};
use crate::contracts::types::DiagnosticRecord;
use crate::exit_codes;
use crate::output::apply_rtk_if_eligible;
use crate::policy::{AuthorizedExecutionPlan, AuthorizedProjectionPlan};

#[derive(Debug, Default, Clone, Copy)]
pub struct TokioExecutor;

#[async_trait]
impl Executor for TokioExecutor {
    async fn execute(
        &self,
        plan: &AuthorizedExecutionPlan,
        options: &ExecutionOptions,
    ) -> ExecutionReport {
        match execute_inner(plan, options, &default_signal_factory).await {
            Ok(report) => report,
            Err(err) => err.into_report(),
        }
    }
}

#[async_trait]
impl super::ProjectionExecutor for TokioExecutor {
    async fn execute_projection(
        &self,
        authorized: &AuthorizedProjectionPlan,
        options: &ExecutionOptions,
    ) -> ExecutionReport {
        match execute_projection_inner(authorized, options, &default_signal_factory).await {
            Ok(report) => report,
            Err(err) => err.into_report(),
        }
    }
}

/// Seam for constructing the controller's signal source, injected so tests can force a
/// post-spawn signal-installation failure deterministically (S6.1-07). Must be `Send + Sync`
/// to be held across the `execute_inner` future's `.await` points.
type SignalFactory = dyn Fn() -> io::Result<Box<dyn SignalSource>> + Send + Sync;

fn default_signal_factory() -> io::Result<Box<dyn SignalSource>> {
    UnixSignalSource::install().map(|s| Box::new(s) as Box<dyn SignalSource>)
}

#[derive(Debug)]
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

    /// A failure discovered after `cmd.spawn()` already returned an OS-assigned child. The
    /// child existed (and may have run before cleanup), so this must never present as
    /// `failed_to_spawn` (S6.1-07).
    fn controller_error(code: &str, message: impl Into<String>, pid: Option<u32>) -> Self {
        Self {
            outcome: "controller_error".into(),
            diagnostics: vec![DiagnosticRecord {
                code: code.into(),
                message: message.into(),
            }],
            spawned: true,
            pid,
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

/// Terminate the process group and reap the child before reporting a post-spawn controller
/// failure, so the record never claims a successful finalize while a child lingers (S6.1-07).
async fn reap_after_controller_failure(
    guard: &mut Option<ProcessGroupGuard>,
    child: &mut tokio::process::Child,
) {
    if let Some(g) = guard.as_mut() {
        g.signal_group(libc::SIGKILL);
        g.disarm();
    }
    let _ = child.wait().await;
}

async fn execute_inner(
    authorized: &AuthorizedExecutionPlan,
    options: &ExecutionOptions,
    signal_factory: &SignalFactory,
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

    spawn_and_reap(
        exe,
        cwd,
        &resolved.argv,
        env,
        options,
        signal_factory,
        Some(authorized),
    )
    .await
}

async fn execute_projection_inner(
    authorized: &AuthorizedProjectionPlan,
    options: &ExecutionOptions,
    signal_factory: &SignalFactory,
) -> Result<ExecutionReport, ExecFailure> {
    let sealed = authorized.plan();

    // Test-only post-seal drift seams (honored only when explicitly requested).
    apply_projection_test_drift(sealed.executable_path(), sealed.cwd_path())?;

    sealed.preflight_identities().map_err(|e| ExecFailure {
        outcome: "failed_to_spawn".into(),
        diagnostics: vec![DiagnosticRecord {
            code: e.code().into(),
            message: e.message(),
        }],
        spawned: false,
        pid: None,
    })?;

    sealed.preflight_sources().map_err(|e| ExecFailure {
        outcome: "failed_to_spawn".into(),
        diagnostics: vec![DiagnosticRecord {
            code: e.code().into(),
            message: e.message(),
        }],
        spawned: false,
        pid: None,
    })?;

    let env = build_child_env(sealed.fixed_env(), sealed.inherited_env_keys()).map_err(
        |e: EnvError| {
            let d = e.diagnostic();
            ExecFailure {
                outcome: "failed_to_spawn".into(),
                diagnostics: vec![d],
                spawned: false,
                pid: None,
            }
        },
    )?;

    // Machine JSON projections never use RTK.
    let options = ExecutionOptions {
        mode: ExecutionMode::Json,
        limits: options.limits.clone(),
        rtk_projected: None,
    };

    spawn_and_reap(
        sealed.executable_path(),
        sealed.cwd_path(),
        sealed.argv(),
        env,
        &options,
        signal_factory,
        None,
    )
    .await
}

fn apply_projection_test_drift(exe: &Path, cwd: &Path) -> Result<(), ExecFailure> {
    if std::env::var_os("TAKOGAMI_TEST_INJECT_EXE_DRIFT").is_some() {
        let _ = fs::remove_file(exe);
    }
    if std::env::var_os("TAKOGAMI_TEST_INJECT_CWD_DRIFT").is_some() {
        let bogey = cwd.with_extension("drifted");
        let _ = fs::rename(cwd, &bogey);
    }
    if std::env::var_os("TAKOGAMI_TEST_INJECT_SOURCE_DRIFT").is_some() {
        // Touch a bound source script if present beside the executable.
        if let Some(pkg_bin) = exe.parent() {
            let common = pkg_bin.parent().map(|p| p.join("lib/common.sh"));
            if let Some(path) = common {
                let _ = fs::write(&path, b"# drifted\n");
            }
        }
    }
    Ok(())
}

async fn spawn_and_reap(
    exe: &Path,
    cwd: &Path,
    argv: &[String],
    env: super::environment::EnvSnapshot,
    options: &ExecutionOptions,
    signal_factory: &SignalFactory,
    lifecycle: Option<&AuthorizedExecutionPlan>,
) -> Result<ExecutionReport, ExecFailure> {
    let limit = options.limits.capture_limit;
    let mut cmd = Command::new(exe);
    cmd.args(argv)
        .current_dir(cwd)
        .env_clear()
        .envs(env.pairs.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

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

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            reap_after_controller_failure(&mut guard, &mut child).await;
            return Err(missing_pipe_failure(pid, "stdout"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => {
            reap_after_controller_failure(&mut guard, &mut child).await;
            return Err(missing_pipe_failure(pid, "stderr"));
        }
    };

    let mut signals = match signal_factory() {
        Ok(s) => s,
        Err(e) => {
            reap_after_controller_failure(&mut guard, &mut child).await;
            return Err(ExecFailure::controller_error(
                "execution_signal",
                format!("failed to install signal handlers: {e}"),
                pid,
            ));
        }
    };

    let (stdout_cap, stderr_cap, status) = match &options.mode {
        ExecutionMode::Json => {
            run_capturing(
                &mut child,
                signals.as_mut(),
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
                signals.as_mut(),
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

    let status = match status {
        Ok(s) => {
            if let Some(g) = guard.as_mut() {
                g.disarm();
            }
            s
        }
        Err(e) => {
            if let Some(g) = guard.as_mut() {
                g.signal_group(libc::SIGKILL);
                g.disarm();
            }
            let _ = child.wait().await;
            return Err(wait_failure(&e, pid));
        }
    };

    let (exit_code, signal, outcome) = map_status(&status);

    let mut diagnostics = env.diagnostics;
    push_stream_diags(&mut diagnostics, "stdout", &stdout_cap);
    push_stream_diags(&mut diagnostics, "stderr", &stderr_cap);

    let (compressor, gain, emitted_output_bytes, stdout_summary, stderr_summary) =
        if let Some(authorized) = lifecycle {
            finalize_output(
                authorized,
                options,
                &stdout_cap,
                &stderr_cap,
                &mut diagnostics,
            )
        } else {
            // Projections: never RTK-compress machine JSON.
            let emitted = stdout_cap.bytes.len() as u64 + stderr_cap.bytes.len() as u64;
            (
                "none".into(),
                None,
                emitted,
                StreamSummary::from_capture(&stdout_cap),
                StreamSummary::from_capture(&stderr_cap),
            )
        };

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

// Keep the original execute_inner body removed — replaced by spawn_and_reap above.
#[allow(dead_code)]
fn _projection_placeholder() {}

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

/// A pipe Tokio should always populate once `Stdio::piped()` was requested and `spawn()`
/// succeeded; a genuine `None` here is unreachable through the public `execute` surface. Kept
/// as its own pure function (S6.1-12 / §7.5 "missing child pipe after spawn") so the exact
/// error shape (`controller_error`, spawned=true, pid preserved) is directly unit-testable.
fn missing_pipe_failure(pid: Option<u32>, label: &str) -> ExecFailure {
    ExecFailure::controller_error("execution_io", format!("child {label} pipe missing"), pid)
}

/// S6.1-12 / §7.5 "wait failure": kept as its own pure function so the error shape produced
/// after the SIGKILL-then-re-await recovery (S6.1-07 / §5.6, above) is directly unit-testable
/// without needing a real, OS-level `wait()` failure (which `tokio::process::Child::wait`
/// cannot be made to produce deterministically through the public API).
fn wait_failure(err: &io::Error, pid: Option<u32>) -> ExecFailure {
    ExecFailure::controller_error(
        "execution_io",
        format!("failed to wait for child: {err}"),
        pid,
    )
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

            // S6.1-05: finalize each stream independently. `stream_or_buffer` already streamed
            // raw bytes live for any stream that is not RTK-eligible or that overflowed its
            // buffer; only a still-buffered, under-limit, RTK-eligible stream is unflushed and
            // needs emitting here. One stream's overflow/encoding must never suppress its peer.
            let projected = options.rtk_projected.as_deref();
            let (out_compressor, out_gain, out_emitted) = finalize_human_stream(
                "stdout",
                *rtk_eligible,
                is_dev,
                &resolved.program,
                &resolved.argv,
                stdout_cap,
                &path_dirs,
                projected,
                diagnostics,
            );
            let (err_compressor, err_gain, err_emitted) = finalize_human_stream(
                "stderr",
                *rtk_eligible,
                is_dev,
                &resolved.program,
                &resolved.argv,
                stderr_cap,
                &path_dirs,
                projected,
                diagnostics,
            );

            let compressor = merge_compressor(&out_compressor, &err_compressor);
            let gain = out_gain.or(err_gain);
            let emitted = out_emitted.saturating_add(err_emitted);
            (
                compressor,
                gain,
                emitted,
                StreamSummary::from_capture(stdout_cap),
                StreamSummary::from_capture(stderr_cap),
            )
        }
    }
}

/// Finalize one human-mode stream independently of its peer (S6.1-05). Returns
/// `(compressor, gain, emitted_bytes)` for this stream alone.
#[allow(clippy::too_many_arguments)]
fn finalize_human_stream(
    label: &'static str,
    rtk_eligible: bool,
    is_dev: bool,
    program: &str,
    argv: &[String],
    cap: &StreamCapture,
    path_dirs: &[PathBuf],
    projected: Option<&Path>,
    diagnostics: &mut Vec<DiagnosticRecord>,
) -> (String, Option<f64>, u64) {
    if !rtk_eligible {
        // `stream_or_buffer` streamed every byte live; nothing left to emit here.
        return ("none".into(), None, cap.total_bytes);
    }
    if cap.truncated {
        // Overflowed mid-drain: `stream_or_buffer` already flushed the retained prefix and
        // streamed the remainder live. Emitting again here would duplicate output.
        return ("unsupported".into(), None, cap.total_bytes);
    }

    // Buffered and never flushed: this is the only place these bytes reach their destination.
    if cap.encoding != StreamEncoding::Utf8 {
        let mut broken = false;
        write_human_bytes(label, &cap.bytes, &mut broken);
        if broken {
            diagnostics.push(DiagnosticRecord {
                code: "broken_pipe".into(),
                message: format!("{label}: output consumer closed while emitting human bytes"),
            });
        }
        return ("none".into(), None, cap.bytes.len() as u64);
    }

    let rtk = apply_rtk_if_eligible(
        true, false, is_dev, program, argv, cap, path_dirs, projected,
    );
    diagnostics.extend(rtk.diagnostics.iter().cloned());
    let mut broken = false;
    write_human_bytes(label, &rtk.emitted, &mut broken);
    if broken {
        diagnostics.push(DiagnosticRecord {
            code: "broken_pipe".into(),
            message: format!("{label}: output consumer closed while emitting human bytes"),
        });
    }
    (rtk.compressor, rtk.gain, rtk.emitted.len() as u64)
}

fn write_human_bytes(label: &str, bytes: &[u8], broken: &mut bool) {
    match label {
        "stdout" => {
            let mut out = io::stdout().lock();
            let _ = write_ignore_pipe(&mut out, bytes, broken);
        }
        "stderr" => {
            let mut err = io::stderr().lock();
            let _ = write_ignore_pipe(&mut err, bytes, broken);
        }
        _ => unreachable!("finalize_human_stream only uses \"stdout\"/\"stderr\" labels"),
    }
}

/// Combine two independent per-stream compressor labels into one summary value, preferring
/// the more specific outcome: an actual `rtk` compression, then an `unsupported` overflow, then
/// `none`.
fn merge_compressor(a: &str, b: &str) -> String {
    for candidate in ["rtk", "unsupported"] {
        if a == candidate || b == candidate {
            return candidate.into();
        }
    }
    "none".into()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::fingerprint_file;
    use crate::policy::PolicyEvaluationResult;
    use crate::registry::{RegistryAccess, RegistryPaths};
    use crate::resolution::{FixedIdGenerator, LifecycleVerb, ResolutionRequest, resolve};
    use tempfile::TempDir;

    /// Builds a real authorized execution plan whose sealed executable is a fast, real
    /// `exit 0` script, so `execute_inner` can spawn an actual OS child deterministically.
    struct DemoHandoff {
        _temp: TempDir,
        input: crate::resolution::PolicyEvaluationInput,
    }

    fn resolve_demo_handoff() -> DemoHandoff {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        fs::create_dir_all(&workspace).unwrap();
        copy_tree(&fixture, &workspace);

        let path_dir = workspace.join("bin");
        fs::create_dir_all(&path_dir).unwrap();
        for name in ["moon", "demo-bin", "rg"] {
            let p = path_dir.join(name);
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let registry = workspace.join("registry");
        write_demo_units(&workspace, &registry);

        let access = RegistryAccess::new(RegistryPaths {
            registry_root: registry,
            workspace_root: workspace,
        });
        let request = ResolutionRequest {
            session_id: "tkg_process_test".into(),
            unit_id: "demo".into(),
            verb: LifecycleVerb::Build,
            explicit_profile: None,
            explain: false,
            execute_requested: false,
        };
        let mut id_gen = FixedIdGenerator {
            id: "tkg_unused".into(),
        };
        let success =
            resolve(&access, request, vec![path_dir], None, &mut id_gen).expect("resolve demo");
        DemoHandoff {
            input: success.policy_evaluation_input(),
            _temp: temp,
        }
    }

    fn write_demo_units(workspace: &Path, registry: &Path) {
        let desc_dir = registry.join("sources/descriptors");
        let mut fps = Vec::new();
        let mut units = Vec::new();
        for entry in fs::read_dir(&desc_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let authored: toml::Value =
                toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let id = authored["id"].as_str().unwrap().to_string();
            if id != "demo" {
                continue;
            }
            let rel = format!(
                "registry/sources/descriptors/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            fps.push(fingerprint_file(&workspace.join(&rel), &rel).unwrap());
            let entrypoints: serde_json::Value =
                serde_json::to_value(authored.get("entrypoints").unwrap()).unwrap();
            let native: serde_json::Value = serde_json::to_value(
                authored
                    .get("native")
                    .and_then(|n| n.get("manifests"))
                    .unwrap(),
            )
            .unwrap();
            units.push(serde_json::json!({
                "id": id,
                "kind": "package",
                "path": "demo",
                "native_manifests": native,
                "entrypoints": entrypoints,
                "source": "central",
                "provides": [],
                "requires": [],
            }));
        }
        let doc = serde_json::json!({
            "generated_at": "2026-07-21T00:00:00Z",
            "registry_generation": {
                "generated_at": "2026-07-21T00:00:00Z",
                "source_fingerprints": fps,
            },
            "summary": {"total": units.len()},
            "units": units,
        });
        fs::write(
            registry.join("units.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
    }

    fn copy_tree(src: &Path, dst: &Path) {
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                fs::create_dir_all(&to).unwrap();
                copy_tree(&entry.path(), &to);
            } else {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::copy(entry.path(), &to).unwrap();
            }
        }
    }

    fn authorized_demo_plan() -> (DemoHandoff, AuthorizedExecutionPlan) {
        let handoff = resolve_demo_handoff();
        let result = crate::policy::evaluate_policy(&handoff.input);
        let PolicyEvaluationResult::Authorized(plan) = result else {
            panic!("expected dual-Allow authorized plan; got {result:?}");
        };
        (handoff, *plan)
    }

    // S6.1-07: a failure discovered strictly after `cmd.spawn()` succeeded (here, the
    // injected signal-source factory) must never be reported as `failed_to_spawn`. The OS
    // already created the child; the record must say so and keep the PID.
    #[tokio::test]
    async fn post_spawn_signal_source_failure_reports_controller_error_with_pid() {
        let (_handoff, plan) = authorized_demo_plan();
        let options = ExecutionOptions {
            mode: ExecutionMode::Json,
            limits: Default::default(),
            rtk_projected: None,
        };
        let failing_factory: &SignalFactory =
            &|| Err(io::Error::other("injected signal source failure"));

        let report = match execute_inner(&plan, &options, failing_factory).await {
            Ok(report) => report,
            Err(err) => err.into_report(),
        };

        assert_eq!(
            report.outcome, "controller_error",
            "post-spawn failures must not present as failed_to_spawn"
        );
        assert!(
            report.spawned,
            "the child was spawned before the injected failure"
        );
        assert!(
            report.pid.is_some(),
            "the OS-assigned pid must be preserved on a post-spawn failure"
        );
    }

    // Baseline: with the real signal factory, the same plan spawns and completes normally,
    // proving the injected-failure test above is exercising the intended seam and not some
    // unrelated spawn defect.
    #[tokio::test]
    async fn post_spawn_success_path_still_completes() {
        let (_handoff, plan) = authorized_demo_plan();
        let options = ExecutionOptions {
            mode: ExecutionMode::Json,
            limits: Default::default(),
            rtk_projected: None,
        };

        let report = match execute_inner(&plan, &options, &default_signal_factory).await {
            Ok(report) => report,
            Err(err) => err.into_report(),
        };

        assert_eq!(report.outcome, "completed");
        assert!(report.spawned);
        assert!(report.pid.is_some());
    }

    // §7.5: "executable removed before spawn" / "execute permission removed before spawn" /
    // "cwd identity drift" are TOCTOU races against the resolution-time seal. Exercising the
    // real `--execute` CLI path cannot deterministically land the file mutation inside the
    // narrow window between resolution and spawn, so these are proven directly against
    // `preflight_identity`, the exact fail-closed gate `execute_inner` calls first (S6.1-07
    // hardened this path; it must never let a drifted sealed path reach `Command::spawn`).

    fn preflight_workspace() -> (TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("sealed-exe");
        fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let cwd = temp.path().join("sealed-cwd");
        fs::create_dir_all(&cwd).unwrap();
        let exe = exe.canonicalize().unwrap();
        let cwd = cwd.canonicalize().unwrap();
        (temp, exe, cwd)
    }

    #[test]
    fn preflight_identity_accepts_a_stable_sealed_pair() {
        let (_temp, exe, cwd) = preflight_workspace();
        preflight_identity(&exe, &cwd).expect("unchanged sealed executable/cwd must pass");
    }

    #[test]
    fn preflight_identity_rejects_executable_removed_before_spawn() {
        let (_temp, exe, cwd) = preflight_workspace();
        fs::remove_file(&exe).unwrap();
        let err = preflight_identity(&exe, &cwd)
            .expect_err("a removed sealed executable must fail closed");
        assert_eq!(err.outcome, "failed_to_spawn");
        assert!(!err.spawned);
        assert!(
            err.diagnostics
                .iter()
                .any(|d| d.code == "execution_contract"),
            "{:?}",
            err.diagnostics
        );
    }

    #[test]
    fn preflight_identity_rejects_execute_permission_removed_before_spawn() {
        let (_temp, exe, cwd) = preflight_workspace();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o644)).unwrap();
        let err = preflight_identity(&exe, &cwd)
            .expect_err("a non-executable sealed path must fail closed");
        assert_eq!(err.outcome, "failed_to_spawn");
        assert!(!err.spawned);
        assert!(
            err.diagnostics
                .iter()
                .any(|d| d.code == "execution_contract"),
            "{:?}",
            err.diagnostics
        );
    }

    #[test]
    fn preflight_identity_rejects_cwd_removed_before_spawn() {
        let (_temp, exe, cwd) = preflight_workspace();
        fs::remove_dir(&cwd).unwrap();
        let err =
            preflight_identity(&exe, &cwd).expect_err("a removed sealed cwd must fail closed");
        assert_eq!(err.outcome, "failed_to_spawn");
        assert!(
            err.diagnostics
                .iter()
                .any(|d| d.code == "execution_contract"),
            "{:?}",
            err.diagnostics
        );
    }

    // §7.5 "missing child pipe after spawn" / "wait failure": both conditions are unreachable
    // through the public `execute` surface (Tokio always populates `Stdio::piped()` pipes on a
    // successful spawn; `Child::wait` cannot be forced to fail deterministically), so the exact
    // error-shape construction is proven directly against the extracted pure functions instead.

    #[test]
    fn missing_pipe_failure_reports_controller_error_with_pid_and_label() {
        let err = missing_pipe_failure(Some(4242), "stdout");
        assert_eq!(err.outcome, "controller_error");
        assert!(
            err.spawned,
            "the child existed; this must never be failed_to_spawn"
        );
        assert_eq!(err.pid, Some(4242));
        assert!(
            err.diagnostics
                .iter()
                .any(|d| d.code == "execution_io" && d.message.contains("stdout pipe missing")),
            "{:?}",
            err.diagnostics
        );
    }

    #[test]
    fn wait_failure_reports_controller_error_with_pid_and_underlying_message() {
        let os_err = io::Error::other("simulated wait() failure");
        let err = wait_failure(&os_err, Some(777));
        assert_eq!(err.outcome, "controller_error");
        assert!(err.spawned);
        assert_eq!(err.pid, Some(777));
        assert!(
            err.diagnostics.iter().any(|d| d.code == "execution_io"
                && d.message.contains("failed to wait for child")
                && d.message.contains("simulated wait() failure")),
            "{:?}",
            err.diagnostics
        );
    }

    #[test]
    fn preflight_identity_rejects_cwd_replaced_by_a_file() {
        let (_temp, exe, cwd) = preflight_workspace();
        fs::remove_dir(&cwd).unwrap();
        fs::write(&cwd, b"not a directory anymore").unwrap();
        let err = preflight_identity(&exe, &cwd)
            .expect_err("a sealed cwd replaced by a regular file must fail closed");
        assert_eq!(err.outcome, "failed_to_spawn");
        assert!(
            err.diagnostics
                .iter()
                .any(|d| d.code == "execution_contract"),
            "{:?}",
            err.diagnostics
        );
    }
}

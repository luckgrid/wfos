//! Command handlers for discovery / query / doctor and lifecycle planning.

use std::path::{Path, PathBuf};

use crate::cli::{BinCleanupMode, BinCommand, Command, GraphFormat, ListTarget, SessionCommand};
use crate::contracts::types::{
    DiagnosticRecord, ExecutionRecord, OutputSummary, RECORD_KIND_COMMAND_EXECUTION, RequestRecord,
    RuntimeCommandRecord, SCHEMA_VERSION,
};
use crate::contracts::{
    ExecutionClass, PolicyDecision, StateHomeInputs, ensure_state_home, resolve_session_state_home,
};
use crate::doctor::{self, DoctorInputs};
use crate::error::{ControllerError, ExecutionDeferredDetails, PolicyOutcomeDetails};
use crate::execution::{
    ExecutionMode, ExecutionOptions, Executor, ProjectionExecutor, TokioExecutor,
};
use crate::output::OutputSink;
use crate::registry::{
    ExternalAdapters, Freshness, ProcessAdapters, ProfileSelection, RefreshKind, RegistryAccess,
    discover_from_scan, filter_tools, filter_units, find_unit, parse_filters,
    resolve_registry_paths,
};
use crate::resolution::{CorrelationIdGenerator, DefaultIdGenerator, ResolutionRequest, resolve};
use crate::sessions::{
    CommandRecordStore, RecordWriter, RuntimeContextEnv, SessionStoreError,
    collect_runtime_context, list_sessions, show_latest_with_diagnostics, show_session,
    utc_now_rfc3339,
};

pub async fn dispatch_implemented(
    command: &Command,
    sink: &OutputSink,
    cli_state_home: Option<&Path>,
    cli_profile: Option<&str>,
) -> Result<u8, ControllerError> {
    match command {
        Command::Doctor => run_doctor(sink, cli_state_home),
        Command::Scan { refresh } => run_scan(sink, *refresh),
        Command::List { target, filters } => run_list(sink, target, filters),
        Command::Info { unit } => run_info(sink, unit),
        Command::Tools => run_tools(sink),
        Command::Interfaces { validate } => run_interfaces(sink, *validate),
        Command::Session { sub } => run_session(sink, sub, cli_state_home, cli_profile),
        Command::Graph { format } => run_graph(sink, *format),
        Command::Bin { sub } => {
            run_bin(
                sink,
                sub,
                cli_profile,
                cli_state_home,
                &TokioExecutor,
                &default_store_factory,
            )
            .await
        }
        Command::Dev { .. } | Command::Build { .. } | Command::Check { .. } => {
            let (verb, unit, explain, execute) =
                command.lifecycle_parts().expect("lifecycle command");
            run_lifecycle_with_executor(
                sink,
                verb,
                unit,
                explain,
                execute,
                cli_profile,
                cli_state_home,
                &TokioExecutor,
                &default_store_factory,
            )
            .await
        }
    }
}

/// Zero-spawn graph projection: load → validate/freshness → render. Never touches executor/records.
fn run_graph(sink: &OutputSink, format: GraphFormat) -> Result<u8, ControllerError> {
    let paths = resolve_registry_paths()?;
    let access = RegistryAccess::new(paths);
    let loaded = access.load_graph()?;
    let freshness = loaded.freshness;
    let doc = loaded.document;
    let graph_value = serde_json::to_value(&doc)
        .map_err(|e| ControllerError::internal(format!("serialize graph: {e}")))?;

    let (fmt_name, rendered): (&str, Option<String>) = match format {
        GraphFormat::Text => (
            "text",
            Some(crate::graph::render_text(&doc, freshness.as_str())),
        ),
        GraphFormat::Dot => ("dot", Some(crate::graph::render_dot(&doc))),
        GraphFormat::Json => ("json", None),
    };

    if sink.json {
        let mut data = serde_json::json!({
            "format": fmt_name,
            "freshness": freshness.as_str(),
            "graph": graph_value,
        });
        if let Some(r) = rendered {
            data["rendered"] = serde_json::Value::String(r);
        }
        return match sink.emit_success("graph", data, Some(freshness), &[]) {
            Ok(code) => Ok(code),
            Err(e) if crate::output::is_broken_pipe(&e) => Ok(crate::exit_codes::SUCCESS),
            Err(e) => Err(ControllerError::internal(format!("emit: {e}"))),
        };
    }

    use std::io::Write;
    let body = match format {
        GraphFormat::Json => {
            serde_json::to_string_pretty(&doc)
                .map_err(|e| ControllerError::internal(format!("serialize graph: {e}")))?
                + "\n"
        }
        GraphFormat::Text | GraphFormat::Dot => rendered.expect("rendered"),
    };
    let _ = sink.no_color;
    match write!(std::io::stdout(), "{body}") {
        Ok(()) => Ok(crate::exit_codes::SUCCESS),
        Err(e) if crate::output::is_broken_pipe(&e) => Ok(crate::exit_codes::SUCCESS),
        Err(e) => Err(ControllerError::internal(format!("write stdout: {e}"))),
    }
}

/// Factory abstraction for opening a record store, injected so lifecycle coordination does not
/// depend on a concrete [`CommandRecordStore`]. Must be `Send + Sync` to cross the `App::run`
/// future boundary.
pub(crate) type StoreFactory =
    dyn Fn(&Path) -> Result<Box<dyn RecordWriter>, SessionStoreError> + Send + Sync;

/// Production record-store factory. Tests inject a fault-injecting factory instead.
fn default_store_factory(path: &Path) -> Result<Box<dyn RecordWriter>, SessionStoreError> {
    CommandRecordStore::open(path).map(|s| Box::new(s) as Box<dyn RecordWriter>)
}

/// Internal coordinator accepting an injected executor (spy in tests; Tokio in production) and
/// an injected record-store factory (fault injection in tests; real store in production).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_lifecycle_with_executor(
    sink: &OutputSink,
    verb: crate::resolution::LifecycleVerb,
    unit_id: &str,
    explain: bool,
    execute: bool,
    cli_profile: Option<&str>,
    cli_state_home: Option<&Path>,
    executor: &dyn Executor,
    open_store: &StoreFactory,
) -> Result<u8, ControllerError> {
    let access = access()?;
    let mut id_gen = DefaultIdGenerator::default();
    let session_id = id_gen.next_id();
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let env_profile = std::env::var("TAKOGAMI_PROFILE").ok();
    // S6.1-11: collect once and thread through, instead of discarding the bounded diagnostic
    // `collect_runtime_context` returns for an invalid opaque ID.
    let (runtime_context, runtime_context_diag) = runtime_context_now();
    let runtime_context_diagnostics: Vec<crate::contracts::DiagnosticRecord> =
        runtime_context_diag.clone().into_iter().collect();

    let request = ResolutionRequest {
        session_id: session_id.clone(),
        unit_id: unit_id.into(),
        verb,
        explicit_profile: cli_profile.map(str::to_string),
        explain,
        execute_requested: execute,
    };

    let success = match resolve(&access, request, path_dirs, env_profile, &mut id_gen) {
        Ok(s) => s,
        Err(mut err) => {
            if err.session_id().is_none()
                && let ControllerError::Resolution {
                    session_id: sid, ..
                } = &mut err
            {
                *sid = Some(session_id.clone());
            }
            return sink
                .emit_error(verb.as_str(), &err)
                .map_err(|e| ControllerError::internal(e.to_string()));
        }
    };

    let policy_input = success.policy_evaluation_input();
    let profile = policy_input.profile().clone();
    let env_state = std::env::var("TAKOGAMI_STATE_HOME").ok();
    let env_xdg = std::env::var("XDG_STATE_HOME").ok();
    let state_home = resolve_session_state_home(StateHomeInputs {
        cli_state_home,
        env_takogami_state_home: env_state.as_deref(),
        profile_session_state_home: profile.session_state_home.as_deref(),
        env_xdg_state_home: env_xdg.as_deref(),
        home_dir: dirs_home(),
    });

    let policy_result = crate::policy::evaluate_policy(&policy_input);
    let authorized = match policy_result {
        crate::policy::PolicyEvaluationResult::Contract(err) => {
            let err = ControllerError::from_policy_contract(*err);
            return sink
                .emit_policy_contract_outcome(
                    verb.as_str(),
                    &err,
                    &success.plan,
                    success.freshness,
                    execute,
                )
                .map_err(|e| ControllerError::internal(e.to_string()));
        }
        crate::policy::PolicyEvaluationResult::Rejected(rejected) => {
            let outcome = match rejected.decision() {
                PolicyDecision::Deny { .. } => "denied",
                PolicyDecision::Gate { .. } => "gated",
                PolicyDecision::Allow { .. } => {
                    return Err(ControllerError::internal(
                        "policy evaluator returned Allow without authorization",
                    ));
                }
            };
            let record = base_record_from_rejected(
                &rejected,
                verb.as_str(),
                execute,
                outcome,
                runtime_context.clone(),
            );
            persist_terminal(open_store, &state_home, &record)?;
            match rejected.decision() {
                PolicyDecision::Deny { reason, .. } => {
                    let err = ControllerError::PolicyDeny {
                        reason: reason.clone(),
                        details: Box::new(PolicyOutcomeDetails::from_rejected(&rejected)),
                    };
                    return sink
                        .emit_policy_outcome(
                            verb.as_str(),
                            &err,
                            &rejected,
                            success.freshness,
                            &runtime_context_diagnostics,
                        )
                        .map_err(|e| ControllerError::internal(e.to_string()));
                }
                PolicyDecision::Gate { reason, .. } => {
                    let err = ControllerError::PolicyGate {
                        reason: reason.clone(),
                        details: Box::new(PolicyOutcomeDetails::from_rejected(&rejected)),
                    };
                    return sink
                        .emit_policy_outcome(
                            verb.as_str(),
                            &err,
                            &rejected,
                            success.freshness,
                            &runtime_context_diagnostics,
                        )
                        .map_err(|e| ControllerError::internal(e.to_string()));
                }
                PolicyDecision::Allow { .. } => unreachable!(),
            }
        }
        crate::policy::PolicyEvaluationResult::Authorized(authorized) => authorized,
    };

    if success.plan.resolved().execution_class != ExecutionClass::Direct {
        let record = base_record_from_authorized(
            &authorized,
            verb.as_str(),
            execute,
            "execution_unavailable",
            true,
            runtime_context.clone(),
            runtime_context_diag.clone(),
        );
        persist_terminal(open_store, &state_home, &record)?;
        let err = ControllerError::ExecutionClassUnavailable {
            message: format!(
                "execution_class={} with provider {:?} is not executable in S6",
                success.plan.resolved().execution_class.as_str(),
                success.plan.resolved().runtime_provider
            ),
            details: Box::new(ExecutionDeferredDetails::from_authorized(&authorized)),
        };
        return sink
            .emit_error_with_explanation(
                verb.as_str(),
                &err,
                Some(&success.explanation),
                Some(success.freshness),
                &runtime_context_diagnostics,
            )
            .map_err(|e| ControllerError::internal(e.to_string()));
    }

    if execute {
        let store = open_store(&state_home).map_err(|e| ControllerError::StateIo {
            message: e.to_string(),
            code: e.code().into(),
        })?;
        let lock = store
            .acquire_lock(&session_id)
            .map_err(|e| ControllerError::StateIo {
                message: e.to_string(),
                code: e.code().into(),
            })?;

        let mut pending = base_record_from_authorized(
            &authorized,
            verb.as_str(),
            true,
            "pending",
            true,
            runtime_context.clone(),
            runtime_context_diag.clone(),
        );
        pending.ended_at = None;
        store
            .write_pending(&pending, &lock)
            .map_err(|e| ControllerError::StateIo {
                message: e.to_string(),
                code: e.code().into(),
            })?;

        let compressor = profile_compressor(&profile);
        let rtk_projected = access
            .load_tools()
            .ok()
            .and_then(|(doc, _)| crate::output::projected_rtk_detect_path(&doc.tools));
        let options = ExecutionOptions {
            mode: if sink.json {
                ExecutionMode::Json
            } else {
                ExecutionMode::Human {
                    rtk_eligible: compressor == "rtk",
                    profile_id: profile.id.clone(),
                }
            },
            limits: Default::default(),
            rtk_projected,
        };
        let report = executor.execute(&authorized, &options).await;

        if report.spawned
            && let Some(pid) = report.pid
        {
            pending.execution.started = true;
            pending.execution.pid = Some(pid);
            store
                .write_final(&pending, &lock)
                .map_err(|e| ControllerError::StateIo {
                    message: e.to_string(),
                    code: e.code().into(),
                })?;
        }

        // Spy / unavailable stand-ins remain non-spawning for S5.2 reachability tests.
        if !report.spawned
            && matches!(
                report.outcome.as_str(),
                "spy_reached" | "execution_unavailable"
            )
        {
            let mut final_rec = pending;
            final_rec.execution.outcome = "execution_unavailable".into();
            final_rec.ended_at = Some(utc_now_rfc3339());
            store
                .write_final(&final_rec, &lock)
                .map_err(|e| ControllerError::StateIo {
                    message: e.to_string(),
                    code: e.code().into(),
                })?;
            let err = ControllerError::ExecutionUnavailable {
                session_id: session_id.clone(),
                details: Box::new(ExecutionDeferredDetails::from_authorized(&authorized)),
            };
            return sink
                .emit_error_with_explanation(
                    verb.as_str(),
                    &err,
                    Some(&success.explanation),
                    Some(success.freshness),
                    &runtime_context_diagnostics,
                )
                .map_err(|e| ControllerError::internal(e.to_string()));
        }

        let mut final_rec = pending;
        final_rec.execution.outcome = report.outcome.clone();
        final_rec.execution.started = report.spawned;
        final_rec.execution.pid = report.pid;
        final_rec.execution.exit_code = report.exit_code;
        final_rec.execution.signal = report.signal.clone();
        final_rec.ended_at = Some(utc_now_rfc3339());
        final_rec.output_summary = OutputSummary {
            stdout_bytes: report.stdout.total_bytes,
            stderr_bytes: report.stderr.total_bytes,
            truncated: report.stdout.truncated || report.stderr.truncated,
            encoding: merge_encoding(&report.stdout.encoding, &report.stderr.encoding),
            compressor: report.compressor.clone(),
        };
        if let Some(diag) = report.diagnostics.first() {
            final_rec.error = Some(diag.clone());
        }
        store
            .write_final(&final_rec, &lock)
            .map_err(|e| ControllerError::StateIo {
                message: e.to_string(),
                code: e.code().into(),
            })?;

        return sink
            .emit_executed(
                verb.as_str(),
                &authorized,
                &success.explanation,
                success.freshness,
                &report,
                &final_rec,
                &runtime_context_diagnostics,
            )
            .map_err(|e| ControllerError::internal(e.to_string()));
    }

    let record = base_record_from_authorized(
        &authorized,
        verb.as_str(),
        false,
        "planned",
        true,
        runtime_context.clone(),
        runtime_context_diag.clone(),
    );
    persist_terminal(open_store, &state_home, &record)?;

    if explain {
        sink.emit_explanation_with_policy(
            verb.as_str(),
            &authorized,
            &success.explanation,
            success.freshness,
            &runtime_context_diagnostics,
        )
        .map_err(|e| ControllerError::internal(e.to_string()))
    } else {
        sink.emit_plan_with_policy(
            verb.as_str(),
            &authorized,
            &success.explanation,
            success.freshness,
            &runtime_context_diagnostics,
        )
        .map_err(|e| ControllerError::internal(e.to_string()))
    }
}

fn dirs_home() -> Option<&'static Path> {
    // ponytail: resolve once via HOME; tests inject state roots via CLI/env.
    static HOME: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| std::env::var_os("HOME").map(PathBuf::from));
    home.as_deref()
}

fn profile_compressor(profile: &crate::registry::ProfileRecord) -> String {
    profile
        .rest
        .get("output_compressor")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_string()
}

fn persist_terminal(
    open_store: &StoreFactory,
    state_home: &Path,
    record: &RuntimeCommandRecord,
) -> Result<(), ControllerError> {
    ensure_state_home(state_home).map_err(|e| ControllerError::StateIo {
        message: e.to_string(),
        code: "state_io".into(),
    })?;
    let store = open_store(state_home).map_err(|e| ControllerError::StateIo {
        message: e.to_string(),
        code: e.code().into(),
    })?;
    store
        .write_terminal_unlocked(record)
        .map_err(|e| ControllerError::StateIo {
            message: e.to_string(),
            code: e.code().into(),
        })
}

fn request_flags(explain: bool, execute: bool) -> Vec<String> {
    let mut flags = Vec::new();
    if explain {
        flags.push("--explain".into());
    }
    if execute {
        flags.push("--execute".into());
    }
    flags
}

fn empty_output() -> OutputSummary {
    OutputSummary {
        stdout_bytes: 0,
        stderr_bytes: 0,
        truncated: false,
        encoding: "utf-8".into(),
        compressor: "none".into(),
    }
}

fn runtime_context_now() -> (
    Option<crate::contracts::RuntimeContext>,
    Option<crate::contracts::DiagnosticRecord>,
) {
    let herdr_workspace_id = std::env::var("HERDR_WORKSPACE_ID").ok();
    let herdr_tab_id = std::env::var("HERDR_TAB_ID").ok();
    let herdr_pane_id = std::env::var("HERDR_PANE_ID").ok();
    let tmux = std::env::var("TMUX").ok();
    let tmux_pane = std::env::var("TMUX_PANE").ok();
    collect_runtime_context(RuntimeContextEnv {
        herdr_workspace_id: herdr_workspace_id.as_deref(),
        herdr_tab_id: herdr_tab_id.as_deref(),
        herdr_pane_id: herdr_pane_id.as_deref(),
        tmux: tmux.as_deref(),
        tmux_pane: tmux_pane.as_deref(),
    })
}

fn base_record_from_authorized(
    authorized: &crate::policy::AuthorizedExecutionPlan,
    command: &str,
    execute: bool,
    outcome: &str,
    include_resolution: bool,
    runtime_context: Option<crate::contracts::RuntimeContext>,
    runtime_context_diag: Option<crate::contracts::DiagnosticRecord>,
) -> RuntimeCommandRecord {
    let plan = authorized.plan();
    let resolved = plan.resolved();
    let ended = outcome != "pending";
    RuntimeCommandRecord {
        schema_version: SCHEMA_VERSION.into(),
        record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
        session_id: resolved.session_id.clone(),
        plan_digest: plan.plan_digest().to_string(),
        parent_session_id: None,
        work_session_id: None,
        runtime_context,
        started_at: utc_now_rfc3339(),
        ended_at: ended.then(utc_now_rfc3339),
        actor: "agent".into(),
        profile_id: resolved.profile_id.clone(),
        request: RequestRecord {
            command: command.into(),
            unit_id: Some(resolved.unit_id.clone()),
            verb: Some(resolved.verb.clone()),
            flags: request_flags(authorized.request().explain_requested, execute),
        },
        resolution: include_resolution.then(|| resolved.clone()),
        policy_decision: authorized.policy_decision().clone(),
        execution: ExecutionRecord {
            started: false,
            pid: None,
            exit_code: None,
            signal: None,
            outcome: outcome.into(),
        },
        source_fingerprints: resolved.registry_generation.source_fingerprints.clone(),
        output_summary: empty_output(),
        // S6.1-11: safe home for the bounded runtime-context diagnostic; nothing more severe
        // has happened yet at plan/pending-record construction time.
        error: runtime_context_diag,
    }
}

fn base_record_from_rejected(
    rejected: &crate::policy::RejectedPolicyOutcome,
    command: &str,
    execute: bool,
    outcome: &str,
    runtime_context: Option<crate::contracts::RuntimeContext>,
) -> RuntimeCommandRecord {
    let plan = rejected.plan();
    let resolved = plan.resolved();
    RuntimeCommandRecord {
        schema_version: SCHEMA_VERSION.into(),
        record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
        session_id: resolved.session_id.clone(),
        plan_digest: plan.plan_digest().to_string(),
        parent_session_id: None,
        work_session_id: None,
        runtime_context,
        started_at: utc_now_rfc3339(),
        ended_at: Some(utc_now_rfc3339()),
        actor: "agent".into(),
        profile_id: resolved.profile_id.clone(),
        request: RequestRecord {
            command: command.into(),
            unit_id: Some(resolved.unit_id.clone()),
            verb: Some(resolved.verb.clone()),
            flags: request_flags(false, execute || rejected.execution_requested()),
        },
        resolution: None,
        policy_decision: rejected.decision().clone(),
        execution: ExecutionRecord {
            started: false,
            pid: None,
            exit_code: None,
            signal: None,
            outcome: outcome.into(),
        },
        source_fingerprints: resolved.registry_generation.source_fingerprints.clone(),
        output_summary: empty_output(),
        error: Some(crate::contracts::DiagnosticRecord {
            code: if outcome == "denied" {
                "policy_deny".into()
            } else {
                "policy_gate".into()
            },
            message: match rejected.decision() {
                PolicyDecision::Deny { reason, .. } | PolicyDecision::Gate { reason, .. } => {
                    reason.clone()
                }
                PolicyDecision::Allow { .. } => "unexpected".into(),
            },
        }),
    }
}

fn merge_encoding(a: &str, b: &str) -> String {
    for candidate in ["binary", "lossy-utf-8", "utf-8"] {
        if a == candidate || b == candidate {
            return candidate.into();
        }
    }
    "utf-8".into()
}

fn run_session(
    sink: &OutputSink,
    sub: &SessionCommand,
    cli_state_home: Option<&Path>,
    cli_profile: Option<&str>,
) -> Result<u8, ControllerError> {
    let access = access()?;
    // S6.1-09: no state directory is opened before profile/root resolution succeeds, and no
    // profile error path silently falls through to XDG/HOME.
    let profiles = access.load_profiles()?;
    let env_profile = std::env::var("TAKOGAMI_PROFILE").ok();
    let profile_home = match profiles.resolve_profile_selection(cli_profile, env_profile.as_deref())
    {
        ProfileSelection::Explicit(id) => {
            let profile = profiles
                .profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| ControllerError::usage(format!("unknown profile: {id}")))?;
            profile.session_state_home.clone()
        }
        ProfileSelection::Default(id) => profiles
            .profiles
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.session_state_home.clone()),
        ProfileSelection::None => None,
    };
    let env_state = std::env::var("TAKOGAMI_STATE_HOME").ok();
    let env_xdg = std::env::var("XDG_STATE_HOME").ok();
    let state_home = resolve_session_state_home(StateHomeInputs {
        cli_state_home,
        env_takogami_state_home: env_state.as_deref(),
        profile_session_state_home: profile_home.as_deref(),
        env_xdg_state_home: env_xdg.as_deref(),
        home_dir: dirs_home(),
    });
    let store = CommandRecordStore::open(&state_home).map_err(|e| ControllerError::StateIo {
        message: e.to_string(),
        code: e.code().into(),
    })?;

    match sub {
        SessionCommand::List { limit } => {
            let (rows, diagnostics) = list_sessions(&store, *limit).map_err(|e| match e {
                crate::sessions::SessionStoreError::Contract(msg) => ControllerError::usage(msg),
                other => ControllerError::StateIo {
                    message: other.to_string(),
                    code: other.code().into(),
                },
            })?;
            let data = serde_json::json!({
                "count": rows.len(),
                "records": rows,
                "diagnostics": diagnostics.skipped,
            });
            let mut human = vec![format!(
                "Record kind: command_execution (count: {})",
                rows.len()
            )];
            for row in &rows {
                human.push(format!(
                    "Session ID: {}  Execution outcome: {}",
                    row.session_id, row.outcome
                ));
            }
            sink.emit_success("session", data, None, &human)
                .map_err(|e| ControllerError::internal(e.to_string()))
        }
        SessionCommand::Show { session_id } => {
            let record = show_session(&store, session_id).map_err(|e| match e {
                crate::sessions::SessionStoreError::InvalidSessionId => {
                    ControllerError::usage("invalid session id")
                }
                crate::sessions::SessionStoreError::NotFound(id) => {
                    ControllerError::not_found(format!("session {id}"))
                }
                crate::sessions::SessionStoreError::Contract(msg) => ControllerError::contract(msg),
                other => ControllerError::StateIo {
                    message: other.to_string(),
                    code: other.code().into(),
                },
            })?;
            emit_session_record(sink, &record, &[])
        }
        SessionCommand::Latest => {
            // S6.1-08: surface the same skipped-record diagnostics `list` reports, including
            // when every record is invalid (malformed-only store must not look silently empty).
            let (maybe_record, diagnostics) =
                show_latest_with_diagnostics(&store).map_err(|e| ControllerError::StateIo {
                    message: e.to_string(),
                    code: e.code().into(),
                })?;
            match maybe_record {
                Some(record) => emit_session_record(sink, &record, &diagnostics.skipped),
                None => {
                    let extras: Vec<DiagnosticRecord> = diagnostics
                        .skipped
                        .iter()
                        .map(|message| DiagnosticRecord {
                            code: "skipped_record".into(),
                            message: message.clone(),
                        })
                        .collect();
                    let err = ControllerError::not_found("no command execution records");
                    sink.emit_error_with_explanation("session", &err, None, None, &extras)
                        .map_err(|e| ControllerError::internal(e.to_string()))
                }
            }
        }
    }
}

fn emit_session_record(
    sink: &OutputSink,
    record: &RuntimeCommandRecord,
    diagnostics: &[String],
) -> Result<u8, ControllerError> {
    let mut data =
        serde_json::to_value(record).map_err(|e| ControllerError::internal(e.to_string()))?;
    if !diagnostics.is_empty()
        && let Some(obj) = data.as_object_mut()
    {
        obj.insert("diagnostics".into(), serde_json::json!(diagnostics));
    }
    let human = vec![
        format!("Record kind: {}", record.record_kind),
        format!("Session ID: {}", record.session_id),
        format!("Execution outcome: {}", record.execution.outcome),
    ];
    sink.emit_success("session", data, None, &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn access() -> Result<RegistryAccess, ControllerError> {
    Ok(RegistryAccess::new(resolve_registry_paths()?))
}

fn run_doctor(sink: &OutputSink, cli_state_home: Option<&Path>) -> Result<u8, ControllerError> {
    let reg = access().ok();
    let report = doctor::run_doctor(DoctorInputs {
        registry: reg.as_ref(),
        cli_state_home,
        path_var: None,
    });
    sink.emit_doctor(&report)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_scan(sink: &OutputSink, refresh: bool) -> Result<u8, ControllerError> {
    let access = access()?;
    if refresh {
        let adapters = ProcessAdapters;
        let cwd = access
            .paths
            .registry_root
            .ancestors()
            .nth(3) // packages/ontarch/registry → wfos
            .unwrap_or(Path::new("."));
        let out = adapters.refresh(RefreshKind::Scan, cwd)?;
        if !out.status.success() {
            return Err(ControllerError::unavailable_source(format!(
                "ontarch scan refresh failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
    }
    let (scan, scan_fresh) = access.load_scan()?;
    let (units, units_fresh) = access.load_units()?;
    let freshness = match (scan_fresh, units_fresh) {
        (Freshness::Miss, Freshness::Miss) => Freshness::Miss,
        (Freshness::Hit, Freshness::Hit) => Freshness::Hit,
        _ => Freshness::Stale,
    };
    let mut units_doc = units;
    if matches!(freshness, Freshness::Miss) {
        units_doc.units = access.source_fallback_units()?;
    }
    let discovery = discover_from_scan(&scan, &units_doc, freshness)?;
    let data = serde_json::json!({
        "freshness": freshness.as_str(),
        "workspaces": discovery.workspaces,
        "units": discovery.units,
        "provisional": discovery.provisional,
        "lint_check_commands_evidence_only": true,
    });
    let human = vec![
        format!("takogami scan (freshness: {})", freshness.as_str()),
        format!("  descriptor-backed units: {}", discovery.units.len()),
        format!(
            "  provisional (descriptor-less): {}",
            discovery.provisional.len()
        ),
        format!("  workspaces: {}", discovery.workspaces.len()),
        "  note: lint_check_commands are evidence only (not executed)".into(),
    ];
    sink.emit_success("scan", data, Some(freshness), &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_list(
    sink: &OutputSink,
    target: &ListTarget,
    filters: &[String],
) -> Result<u8, ControllerError> {
    let access = access()?;
    let parsed = parse_filters(filters)?;
    match target {
        ListTarget::Units => {
            let (mut doc, freshness) = access.load_units()?;
            if freshness == Freshness::Miss {
                doc.units = access.source_fallback_units()?;
            }
            let units = filter_units(&doc.units, &parsed);
            let data = serde_json::json!({
                "freshness": freshness.as_str(),
                "count": units.len(),
                "units": units,
            });
            let mut human = vec![format!(
                "takogami list units (freshness: {}, count: {})",
                freshness.as_str(),
                units.len()
            )];
            for u in &units {
                human.push(format!(
                    "  {}  {}  {}",
                    u.id,
                    u.kind.as_deref().unwrap_or("-"),
                    u.path.as_deref().unwrap_or("-")
                ));
            }
            sink.emit_success("list", data, Some(freshness), &human)
                .map_err(|e| ControllerError::internal(e.to_string()))
        }
        ListTarget::Tools => {
            let (doc, freshness) = access.load_tools()?;
            let tools = filter_tools(&doc.tools, &parsed);
            let data = serde_json::json!({
                "freshness": freshness.as_str(),
                "count": tools.len(),
                "tools": tools,
                "source": "ontarch/tools.json (panoply projection)",
            });
            let mut human = vec![format!("takogami list tools (count: {})", tools.len())];
            for t in &tools {
                human.push(format!(
                    "  {}  module={}  installed={}",
                    t.id,
                    t.module.as_deref().unwrap_or("-"),
                    t.installed
                        .map(|b| if b { "true" } else { "false" })
                        .unwrap_or("-")
                ));
            }
            sink.emit_success("list", data, Some(freshness), &human)
                .map_err(|e| ControllerError::internal(e.to_string()))
        }
    }
}

fn run_info(sink: &OutputSink, unit_id: &str) -> Result<u8, ControllerError> {
    let access = access()?;
    let (mut doc, freshness) = access.load_units()?;
    if freshness == Freshness::Miss {
        doc.units = access.source_fallback_units()?;
    }
    let unit = find_unit(&doc.units, unit_id)?.clone();
    let data = serde_json::json!({
        "freshness": freshness.as_str(),
        "unit": unit,
        "provenance": {
            "source": unit.source,
            "path": unit.path,
            "provisional": unit.provisional,
            "routing_complete": unit.routing_complete,
        }
    });
    let human = vec![
        format!(
            "takogami info {unit_id} (freshness: {})",
            freshness.as_str()
        ),
        format!("  kind: {}", unit.kind.as_deref().unwrap_or("-")),
        format!("  path: {}", unit.path.as_deref().unwrap_or("-")),
        format!("  source: {}", unit.source.as_deref().unwrap_or("-")),
        format!("  provisional: {}", unit.provisional),
    ];
    sink.emit_success("info", data, Some(freshness), &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_tools(sink: &OutputSink) -> Result<u8, ControllerError> {
    let access = access()?;
    let (doc, freshness) = access.load_tools()?;
    let adapters = ProcessAdapters;
    let panoply: Option<serde_json::Value> = adapters.panoply_doctor_json().ok().and_then(|o| {
        if o.status.success() {
            serde_json::from_slice(&o.stdout).ok()
        } else {
            None
        }
    });

    let classified: Vec<_> = doc
        .tools
        .iter()
        .map(|t| {
            let class = if t.default == Some(true) {
                "required"
            } else if matches!(t.id.as_str(), "herdr" | "tmux" | "rtk") {
                "optional"
            } else if t.installed == Some(true) {
                "selected"
            } else {
                "optional"
            };
            serde_json::json!({
                "id": t.id,
                "module": t.module,
                "installed": t.installed,
                "default": t.default,
                "capability_class": class,
                "version": t.version,
            })
        })
        .collect();

    let data = serde_json::json!({
        "freshness": freshness.as_str(),
        "tools": classified,
        "panoply_doctor": panoply,
        "notes": [
            "Tools are projected from Panoply/Ontarch — Takogami does not maintain a second catalog.",
            "Absence of Herdr is never a required failure for base doctor.",
        ],
    });
    let human = vec![
        format!("takogami tools ({} projected)", classified.len()),
        "  source: Ontarch tools.json + optional panoply doctor --json".into(),
    ];
    sink.emit_success("tools", data, Some(freshness), &human)
        .map_err(|e| ControllerError::internal(e.to_string()))
}

fn run_interfaces(sink: &OutputSink, validate: bool) -> Result<u8, ControllerError> {
    let access = access()?;
    let (readable, detail) = access.contracts_readable();
    if validate {
        let adapters = ProcessAdapters;
        let cwd = access
            .paths
            .workspace_root
            .join("Build/src/workspaces/wfos");
        let cwd = if cwd.is_dir() {
            cwd
        } else {
            access.paths.registry_root.clone()
        };
        let out = adapters.validate(&cwd)?;
        let ok = out.status.success();
        let data = serde_json::json!({
            "validate": true,
            "ok": ok,
            "contracts_readable": readable,
            "detail": detail,
            "stdout": String::from_utf8_lossy(&out.stdout),
            "stderr": String::from_utf8_lossy(&out.stderr),
        });
        if sink.json {
            let mut env = crate::contracts::CommandEnvelope::ok("interfaces", Some(data));
            if !ok {
                env.status = "error".into();
                env.exit_code = crate::exit_codes::CONTRACT;
            }
            sink.emit_envelope(&env)
                .map_err(|e| ControllerError::internal(e.to_string()))?;
            Ok(if ok {
                crate::exit_codes::SUCCESS
            } else {
                crate::exit_codes::CONTRACT
            })
        } else {
            writeln_human(&format!(
                "takogami interfaces --validate: {}",
                if ok { "PASS" } else { "FAIL" }
            ))?;
            Ok(if ok {
                crate::exit_codes::SUCCESS
            } else {
                crate::exit_codes::CONTRACT
            })
        }
    } else {
        let data = serde_json::json!({
            "validate": false,
            "contracts_readable": readable,
            "detail": detail,
        });
        let human = vec![
            "takogami interfaces (readability only; pass --validate to run ontarch validate)"
                .into(),
            format!("  contracts_readable: {readable}"),
            format!("  {detail}"),
        ];
        sink.emit_success("interfaces", data, None, &human)
            .map_err(|e| ControllerError::internal(e.to_string()))
    }
}

fn writeln_human(line: &str) -> Result<(), ControllerError> {
    use std::io::Write;
    writeln!(std::io::stdout(), "{line}").map_err(|e| ControllerError::internal(e.to_string()))
}

async fn run_bin(
    sink: &OutputSink,
    sub: &BinCommand,
    cli_profile: Option<&str>,
    cli_state_home: Option<&Path>,
    executor: &dyn ProjectionExecutor,
    open_store: &StoreFactory,
) -> Result<u8, ControllerError> {
    use crate::bin_projection::{CleanupMode, decode_cleanup_plan, decode_inventory};
    use crate::contracts::{OutputSummary, SCHEMA_VERSION};
    use crate::policy::{
        ProjectionEvaluationInput, ProjectionEvaluationResult, evaluate_projection_policy,
    };
    use crate::projection::{
        ProjectionOperation, ScopeError, SealedProjectionPlan, ValidatedBinScope,
    };
    use crate::resolution::profile::{collect_bin_policy_refs, select_profile};
    use crate::resolution::{CorrelationIdGenerator, DefaultIdGenerator};

    let (operation, raw_scope) = match sub {
        BinCommand::Report => (ProjectionOperation::BinReport, None),
        BinCommand::Cleanup { mode, scope } => {
            let op = match mode {
                BinCleanupMode::ReportOnly => ProjectionOperation::BinCleanupReportOnly,
                BinCleanupMode::DryRun => ProjectionOperation::BinCleanupDryRun,
                BinCleanupMode::Archive => ProjectionOperation::BinCleanupArchive,
                BinCleanupMode::DeleteApproved => ProjectionOperation::BinCleanupDeleteApproved,
            };
            (op, scope.as_deref())
        }
    };

    let scope = match raw_scope {
        None => None,
        Some(raw) => match ValidatedBinScope::parse(raw) {
            Ok(s) => Some(s),
            Err(_) => {
                let err = ControllerError::usage(format!(
                    "{} ({})",
                    ScopeError::Invalid.message(),
                    ScopeError::Invalid.code()
                ));
                return sink
                    .emit_error("bin", &err)
                    .map_err(|e| ControllerError::internal(e.to_string()));
            }
        },
    };

    let paths = resolve_registry_paths()?;
    let access = RegistryAccess::new(paths.clone());
    let profiles = access
        .load_profiles()
        .map_err(|e| ControllerError::contract(format!("profile registry unavailable: {e}")))?;
    let profile = select_profile(
        &profiles,
        cli_profile,
        std::env::var("TAKOGAMI_PROFILE").ok().as_deref(),
    )
    .map_err(|e| ControllerError::contract(format!("profile selection failed: {e:?}")))?;
    let policies_doc = access
        .load_policies()
        .map_err(|e| ControllerError::contract(format!("policy registry unavailable: {e}")))?;
    let selected = collect_bin_policy_refs(&policies_doc, &profile)
        .map_err(|e| ControllerError::contract(format!("policy selection failed: {e:?}")))?;

    let mut id_gen = DefaultIdGenerator::default();
    let session_id = id_gen.next_id();
    let policy_root = paths
        .workspace_root
        .canonicalize()
        .map_err(|e| ControllerError::contract(format!("cannot canonicalize policy root: {e}")))?;

    let sealed = SealedProjectionPlan::seal(
        operation,
        &paths.registry_root,
        &paths.workspace_root,
        scope,
        session_id.clone(),
        selected.profile.id.clone(),
        selected.policy_ids.clone(),
    )
    .map_err(|e| ControllerError::ExecutionIo {
        message: e.message(),
        code: e.code().into(),
    })?;

    let eval_input = ProjectionEvaluationInput::new(
        sealed.clone(),
        selected.profile.clone(),
        selected.policies.clone(),
        selected.policy_origins.clone(),
        policy_root,
    );
    let eval = evaluate_projection_policy(&eval_input);

    let command_name = operation.request_command_name();
    let env_state = std::env::var("TAKOGAMI_STATE_HOME").ok();
    let env_xdg = std::env::var("XDG_STATE_HOME").ok();
    let state_home = resolve_session_state_home(StateHomeInputs {
        cli_state_home,
        env_takogami_state_home: env_state.as_deref(),
        profile_session_state_home: selected.profile.session_state_home.as_deref(),
        env_xdg_state_home: env_xdg.as_deref(),
        home_dir: dirs_home(),
    });

    match eval {
        ProjectionEvaluationResult::Contract(err) => {
            let kind_code = err.kind.code();
            let ctrl = ControllerError::PolicyContract {
                code: kind_code.into(),
                message: err.message.clone(),
                details: Box::new(crate::error::PolicyContractDetails {
                    code: kind_code.into(),
                    message: err.message.clone(),
                    session_id: err.session_id.clone(),
                    plan_digest: err.plan_digest.clone(),
                    policy_id: err.policy_id.clone(),
                    field: err.field.clone(),
                }),
            };
            sink.emit_error("bin", &ctrl)
                .map_err(|e| ControllerError::internal(e.to_string()))
        }
        ProjectionEvaluationResult::Rejected(rejected) => {
            let outcome = match rejected.decision() {
                PolicyDecision::Deny { .. } => "denied",
                PolicyDecision::Gate { .. } => "gated",
                PolicyDecision::Allow { .. } => {
                    return Err(ControllerError::internal(
                        "projection evaluator returned Allow without authorization",
                    ));
                }
            };
            let mut record = projection_terminal_record(
                rejected.plan(),
                rejected.decision().clone(),
                outcome,
                false,
                None,
                None,
                None,
            );
            if rejected.deferred_unavailable() {
                record.error = Some(DiagnosticRecord {
                    code: "deferred_unavailable".into(),
                    message: "archive/delete-approved remain deferred; no child spawn".into(),
                });
            } else {
                record.error = Some(DiagnosticRecord {
                    code: if outcome == "denied" {
                        "policy_deny".into()
                    } else {
                        "policy_gate".into()
                    },
                    message: format!("{outcome} by policy"),
                });
            }
            persist_terminal(open_store, &state_home, &record)?;

            let exit = match rejected.decision() {
                PolicyDecision::Deny { .. } => crate::exit_codes::POLICY_DENY,
                PolicyDecision::Gate { .. } => crate::exit_codes::POLICY_GATE,
                PolicyDecision::Allow { .. } => unreachable!(),
            };
            let mut data = serde_json::json!({
                "policy_decision": rejected.decision(),
                "plan_digest": rejected.plan().plan_digest(),
                "session_id": rejected.plan().session_id(),
            });
            if rejected.deferred_unavailable() {
                data["deferred_unavailable"] = serde_json::json!(true);
                data["diagnostics"] = serde_json::json!([{
                    "code": "deferred_unavailable",
                    "message": "archive/delete-approved remain deferred; no child spawn"
                }]);
            }
            let status = if exit == crate::exit_codes::POLICY_GATE {
                "gated"
            } else {
                "denied"
            };
            let diagnostics = if rejected.deferred_unavailable() {
                vec![DiagnosticRecord {
                    code: "deferred_unavailable".into(),
                    message: "archive/delete-approved remain deferred; no child spawn".into(),
                }]
            } else {
                vec![]
            };
            let envelope = crate::contracts::CommandEnvelope {
                schema_version: SCHEMA_VERSION.into(),
                command: command_name.into(),
                session_id: Some(rejected.plan().session_id().into()),
                status: status.into(),
                exit_code: exit,
                data: Some(data),
                explanation: None,
                diagnostics: diagnostics.clone(),
                child: None,
                metrics: None,
            };
            emit_bin_outcome(sink, exit, &envelope, || {
                crate::output::render_bin_policy_human(
                    command_name,
                    status,
                    rejected.deferred_unavailable(),
                    rejected.plan().safe_scope().map(|s| s.as_str()),
                )
            })
        }
        ProjectionEvaluationResult::Authorized(authorized) => {
            let plan = authorized.plan();
            let store = open_store(&state_home).map_err(|e| ControllerError::StateIo {
                message: e.to_string(),
                code: e.code().into(),
            })?;
            let lock =
                store
                    .acquire_lock(plan.session_id())
                    .map_err(|e| ControllerError::StateIo {
                        message: e.to_string(),
                        code: e.code().into(),
                    })?;

            let mut pending = projection_terminal_record(
                plan,
                authorized.policy_decision().clone(),
                "pending",
                false,
                None,
                None,
                None,
            );
            pending.ended_at = None;
            store
                .write_pending(&pending, &lock)
                .map_err(|e| ControllerError::StateIo {
                    message: e.to_string(),
                    code: e.code().into(),
                })?;

            let report = executor
                .execute_projection(&authorized, &crate::execution::ExecutionOptions::default())
                .await;

            if report.spawned {
                let mut pid_pending = pending.clone();
                pid_pending.execution.started = true;
                pid_pending.execution.pid = report.pid;
                if let Err(e) = store.write_final(&pid_pending, &lock) {
                    // PID-pending failure still propagates; child may have started.
                    drop(lock);
                    return Err(ControllerError::StateIo {
                        message: e.to_string(),
                        code: e.code().into(),
                    });
                }
            }

            let mut outcome = report.outcome.clone();
            let mut exit = report.exit_code.unwrap_or(crate::exit_codes::EXECUTION_IO);
            let mut payload_value = None;
            let mut diagnostics = report.diagnostics.clone();
            let mut controller_error = None;

            if report.spawned && report.signal.is_none() && report.exit_code == Some(0) {
                let stdout_bytes = report.stdout.bytes.clone();
                let truncated = report.stdout.truncated;
                let validate_result = match operation {
                    ProjectionOperation::BinReport => {
                        decode_inventory(&stdout_bytes, truncated, &paths.workspace_root)
                            .map(|inv| serde_json::to_value(inv).unwrap())
                    }
                    ProjectionOperation::BinCleanupReportOnly => decode_cleanup_plan(
                        &stdout_bytes,
                        truncated,
                        CleanupMode::ReportOnly,
                        plan.safe_scope(),
                    )
                    .map(|p| serde_json::to_value(p).unwrap()),
                    _ => unreachable!("authorized path only for child-supported ops"),
                };
                match validate_result {
                    Ok(v) => payload_value = Some(v),
                    Err(pe) => {
                        outcome = "controller_error".into();
                        exit = crate::exit_codes::CONTRACT;
                        diagnostics.push(DiagnosticRecord {
                            code: pe.code().into(),
                            message: pe.message(),
                        });
                        controller_error = Some(DiagnosticRecord {
                            code: pe.code().into(),
                            message: pe.message(),
                        });
                    }
                }
            } else if report.spawned && report.exit_code != Some(0) {
                // Preserve native nonzero exit.
                exit = report.exit_code.unwrap_or(1);
            } else if !report.spawned {
                exit = crate::exit_codes::EXECUTION_IO;
                outcome = report.outcome;
            }

            // Derive terminal from last successfully installed record identity.
            let mut terminal = pending;
            if report.spawned {
                terminal.execution.started = true;
                terminal.execution.pid = report.pid;
            }
            terminal.execution.started = report.spawned;
            terminal.execution.pid = report.pid;
            terminal.execution.exit_code = report.exit_code;
            terminal.execution.signal = report.signal.clone();
            terminal.execution.outcome = outcome.clone();
            terminal.ended_at = Some(utc_now_rfc3339());
            terminal.output_summary = OutputSummary {
                stdout_bytes: report.stdout.total_bytes,
                stderr_bytes: report.stderr.total_bytes,
                truncated: report.stdout.truncated || report.stderr.truncated,
                encoding: merge_encoding(&report.stdout.encoding, &report.stderr.encoding),
                compressor: report.compressor.clone(),
            };
            if controller_error.is_some() {
                terminal.error = controller_error;
            } else if let Some(diag) = diagnostics.first() {
                terminal.error = Some(diag.clone());
            }
            if let Err(e) = store.write_final(&terminal, &lock) {
                drop(lock);
                return Err(ControllerError::StateIo {
                    message: e.to_string(),
                    code: e.code().into(),
                });
            }
            drop(lock);

            let data = serde_json::json!({
                "policy_decision": authorized.policy_decision(),
                "plan_digest": plan.plan_digest(),
                "session_id": plan.session_id(),
                "execution": {
                    "outcome": outcome,
                    "started": report.spawned,
                    "pid": report.pid,
                    "exit_code": report.exit_code,
                    "signal": report.signal,
                },
                "payload": payload_value,
            });
            let status = if exit == 0 { "ok" } else { "error" };
            let envelope = crate::contracts::CommandEnvelope {
                schema_version: SCHEMA_VERSION.into(),
                command: command_name.into(),
                session_id: Some(plan.session_id().into()),
                status: status.into(),
                exit_code: exit,
                data: Some(data.clone()),
                explanation: None,
                diagnostics,
                child: None,
                metrics: None,
            };
            let payload_for_human = data.get("payload").cloned();
            emit_bin_outcome(sink, exit, &envelope, || {
                crate::output::render_bin_allow_human(
                    operation,
                    plan.safe_scope().map(|s| s.as_str()),
                    payload_for_human.as_ref(),
                    exit,
                )
            })
        }
    }
}

fn emit_bin_outcome(
    sink: &OutputSink,
    exit: u8,
    envelope: &crate::contracts::CommandEnvelope,
    human: impl FnOnce() -> Vec<String>,
) -> Result<u8, ControllerError> {
    let result = if sink.json {
        sink.emit_envelope(envelope)
    } else {
        let mut out = String::new();
        for line in human() {
            out.push_str(&line);
            out.push('\n');
        }
        use std::io::Write;
        write!(std::io::stdout(), "{out}")
    };
    match result {
        Ok(()) => Ok(exit),
        Err(e) if crate::output::is_broken_pipe(&e) => Ok(exit),
        Err(e) => Err(ControllerError::internal(e.to_string())),
    }
}

fn projection_terminal_record(
    plan: &crate::projection::SealedProjectionPlan,
    decision: PolicyDecision,
    outcome: &str,
    started: bool,
    pid: Option<u32>,
    exit_code: Option<u8>,
    signal: Option<String>,
) -> RuntimeCommandRecord {
    use crate::sessions::utc_now_rfc3339;
    let ended = outcome != "pending";
    RuntimeCommandRecord {
        schema_version: SCHEMA_VERSION.into(),
        record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
        session_id: plan.session_id().into(),
        plan_digest: plan.plan_digest().into(),
        parent_session_id: None,
        work_session_id: None,
        runtime_context: None,
        started_at: utc_now_rfc3339(),
        ended_at: ended.then(utc_now_rfc3339),
        actor: "agent".into(),
        profile_id: plan.profile_id().into(),
        request: plan.safe_request().clone(),
        resolution: None,
        policy_decision: decision,
        execution: ExecutionRecord {
            started,
            pid,
            exit_code,
            signal,
            outcome: outcome.into(),
        },
        source_fingerprints: plan.source_fingerprints().to_vec(),
        output_summary: empty_output(),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{RegistryGeneration, fingerprint_file};
    use crate::execution::{
        ExecutionReport, HelperShadowSpec, MutatingProjectionExecutor, MutationKind,
        ProjectionExecutor, ProjectionTestMutation, SpyExecutor, SpyProjectionExecutor,
        UnavailableExecutor,
    };
    use crate::exit_codes::{
        CONTRACT, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, RESOLUTION, SUCCESS,
    };
    use crate::resolution::LifecycleVerb;
    use crate::sessions::SessionLock;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        workspace: PathBuf,
        registry: PathBuf,
        path_dir: PathBuf,
        marker: PathBuf,
        state_home: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Fixture {
        fn new() -> Self {
            // Recover from poisoning: a panic in one test while holding this lock must not
            // cascade into spurious failures for every other test sharing this process.
            let guard = env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let fixture =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resolution");
            let temp = tempfile::tempdir().unwrap();
            let workspace = temp.path().join("ws");
            let state_home = temp.path().join("state");
            fs::create_dir_all(&workspace).unwrap();
            fs::create_dir_all(&state_home).unwrap();
            copy_tree(&fixture, &workspace);
            let path_dir = workspace.join("bin");
            fs::create_dir_all(&path_dir).unwrap();
            let marker = temp.path().join("marker");
            for name in ["moon", "demo-bin", "rg", "ontarch", "rm"] {
                write_marker_exe(&path_dir.join(name), &marker);
            }
            let registry = workspace.join("registry");
            let fx = Self {
                _temp: temp,
                workspace: workspace.clone(),
                registry,
                path_dir: path_dir.clone(),
                marker,
                state_home: state_home.clone(),
                _guard: guard,
            };
            fx.write_hit_units();
            unsafe {
                std::env::set_var(
                    "TAKOGAMI_ONTARCH_REGISTRY",
                    fx.registry.display().to_string(),
                );
                std::env::set_var(
                    "TAKOGAMI_WORKSPACE_ROOT",
                    fx.workspace.display().to_string(),
                );
                std::env::set_var("PATH", fx.path_dir.display().to_string());
                std::env::set_var("TAKOGAMI_STATE_HOME", fx.state_home.display().to_string());
                std::env::remove_var("TAKOGAMI_PROFILE");
            }
            fx
        }

        fn write_hit_units(&self) {
            let desc_dir = self.registry.join("sources/descriptors");
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
                let rel = format!(
                    "registry/sources/descriptors/{}",
                    path.file_name().unwrap().to_string_lossy()
                );
                fps.push(fingerprint_file(&self.workspace.join(&rel), &rel).unwrap());
                let entrypoints: serde_json::Value =
                    serde_json::to_value(authored.get("entrypoints").unwrap()).unwrap();
                let native: serde_json::Value = serde_json::to_value(
                    authored
                        .get("native")
                        .and_then(|n| n.get("manifests"))
                        .unwrap_or(&toml::Value::Array(vec![])),
                )
                .unwrap();
                units.push(serde_json::json!({
                    "id": id,
                    "kind": "package",
                    "path": id,
                    "native_manifests": native,
                    "entrypoints": entrypoints,
                    "source": "central",
                    "provides": [],
                    "requires": [],
                }));
            }
            let meta = RegistryGeneration {
                generated_at: "2026-07-21T00:00:00Z".into(),
                source_fingerprints: fps,
            };
            let doc = serde_json::json!({
                "generated_at": meta.generated_at,
                "registry_generation": meta,
                "summary": {"total": units.len()},
                "units": units,
            });
            fs::write(
                self.registry.join("units.json"),
                serde_json::to_string_pretty(&doc).unwrap(),
            )
            .unwrap();
        }

        fn patch_demo_gated(&mut self) {
            let path = self
                .registry
                .join("sources/descriptors/demo.descriptor.toml");
            let text = fs::read_to_string(&path).unwrap().replace(
                r#"program = "moon"
args = ["run", "demo:build"]
cwd = "demo"
env_keys = ["PATH"]
backend = "moon"
adapter = "moon-task""#,
                r#"program = "ontarch"
args = ["bin-cleanup", "--mode", "dry-run"]
cwd = "demo"
env_keys = ["PATH"]
backend = "native"
adapter = "direct""#,
            );
            fs::write(&path, text).unwrap();
            self.write_hit_units();
        }

        fn patch_request_policy(&self, effect: &str) {
            let path = self.registry.join("policies.json");
            let mut document: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let policies = document["policies"].as_array_mut().unwrap();
            let request_policy = policies
                .iter_mut()
                .find(|policy| policy["id"] == "takogami.agent")
                .unwrap();
            let allow = request_policy["allow"]["commands"].as_array_mut().unwrap();
            allow.retain(|command| command.as_str() != Some("takogami build"));
            request_policy[effect]["commands"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::Value::String("takogami build".into()));
            fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();
        }

        fn patch_policy_contract_invalid(&self) {
            let path = self.registry.join("profiles.json");
            let mut document: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let profiles = document["profiles"].as_array_mut().unwrap();
            let profile = profiles
                .iter_mut()
                .find(|profile| profile["id"] == "workspace-dev")
                .unwrap();
            profile["allowed_commands"] = serde_json::Value::Null;
            fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();
        }

        fn assert_marker_untouched(&self) {
            assert!(!self.marker.exists(), "marker must never run");
        }

        fn load_state_home_records(&self) -> Vec<serde_json::Value> {
            if !self.state_home.exists() {
                return Vec::new();
            }
            let mut out = Vec::new();
            for entry in fs::read_dir(&self.state_home).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || !name.ends_with(".json") {
                    continue;
                }
                out.push(serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap());
            }
            out
        }
    }

    fn write_marker_exe(path: &Path, marker: &Path) {
        let script = format!("#!/bin/sh\necho ran >> {}\nexit 0\n", marker.display());
        fs::write(path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
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

    async fn run_with_executor(
        unit: &str,
        execute: bool,
        profile: Option<&str>,
        executor: &dyn Executor,
    ) -> Result<u8, ControllerError> {
        run_with_executor_and_store(unit, execute, profile, executor, &default_store_factory).await
    }

    async fn run_with_executor_and_store(
        unit: &str,
        execute: bool,
        profile: Option<&str>,
        executor: &dyn Executor,
        open_store: &StoreFactory,
    ) -> Result<u8, ControllerError> {
        let sink = OutputSink {
            json: true,
            no_color: true,
        };
        run_lifecycle_with_executor(
            &sink,
            LifecycleVerb::Build,
            unit,
            false,
            execute,
            profile,
            None,
            executor,
            open_store,
        )
        .await
    }

    async fn run(
        unit: &str,
        execute: bool,
        profile: Option<&str>,
        spy: &SpyExecutor,
    ) -> Result<u8, ControllerError> {
        run_with_executor(unit, execute, profile, spy).await
    }

    #[derive(Default)]
    struct RecordingUnavailableExecutor {
        calls: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl Executor for RecordingUnavailableExecutor {
        async fn execute(
            &self,
            plan: &crate::policy::AuthorizedExecutionPlan,
            options: &crate::execution::ExecutionOptions,
        ) -> crate::execution::ExecutionReport {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            UnavailableExecutor.execute(plan, options).await
        }
    }

    // --- S6.1-01 / S6.1-02 regressions: every lifecycle record write must be observable. ---

    /// Test double returning a fixed [`ExecutionReport`] without spawning anything.
    struct ScriptedExecutor {
        calls: AtomicU32,
        report: ExecutionReport,
    }

    impl ScriptedExecutor {
        fn new(report: ExecutionReport) -> Self {
            Self {
                calls: AtomicU32::new(0),
                report,
            }
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Executor for ScriptedExecutor {
        async fn execute(
            &self,
            _plan: &crate::policy::AuthorizedExecutionPlan,
            _options: &crate::execution::ExecutionOptions,
        ) -> ExecutionReport {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.report.clone()
        }
    }

    fn scripted_spawned_report(outcome: &str, pid: u32) -> ExecutionReport {
        let mut report = ExecutionReport::idle(outcome);
        report.spawned = true;
        report.pid = Some(pid);
        report.exit_code = Some(0);
        report
    }

    /// Which [`RecordWriter`] call a [`FaultyStore`] should fail, by lifecycle position.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FaultPoint {
        InitialPending,
        FirstFinal,
        SecondFinal,
        TerminalUnlocked,
    }

    /// Wraps a real [`CommandRecordStore`] but injects a deterministic write failure at one
    /// lifecycle position, per S6.1 Â§7.1 ("Use injectable store faults ... do not depend solely
    /// on permission bits").
    struct FaultyStore {
        inner: CommandRecordStore,
        fault: FaultPoint,
        final_calls: AtomicU32,
    }

    impl FaultyStore {
        fn new(inner: CommandRecordStore, fault: FaultPoint) -> Self {
            Self {
                inner,
                fault,
                final_calls: AtomicU32::new(0),
            }
        }

        fn injected(what: &str) -> SessionStoreError {
            SessionStoreError::Contract(format!("injected fault: {what}"))
        }
    }

    impl RecordWriter for FaultyStore {
        fn acquire_lock(&self, session_id: &str) -> Result<SessionLock, SessionStoreError> {
            self.inner.acquire_lock(session_id)
        }

        fn write_pending(
            &self,
            record: &RuntimeCommandRecord,
            lock: &SessionLock,
        ) -> Result<(), SessionStoreError> {
            if self.fault == FaultPoint::InitialPending {
                return Err(Self::injected("initial pending install"));
            }
            self.inner.write_pending(record, lock)
        }

        fn write_final(
            &self,
            record: &RuntimeCommandRecord,
            lock: &SessionLock,
        ) -> Result<(), SessionStoreError> {
            let call_index = self.final_calls.fetch_add(1, Ordering::SeqCst);
            let should_fail = match self.fault {
                FaultPoint::FirstFinal => call_index == 0,
                FaultPoint::SecondFinal => call_index == 1,
                _ => false,
            };
            if should_fail {
                return Err(Self::injected("final replace"));
            }
            self.inner.write_final(record, lock)
        }

        fn write_terminal_unlocked(
            &self,
            record: &RuntimeCommandRecord,
        ) -> Result<(), SessionStoreError> {
            if self.fault == FaultPoint::TerminalUnlocked {
                return Err(Self::injected("terminal unlocked install"));
            }
            self.inner.write_terminal_unlocked(record)
        }
    }

    fn faulty_factory(
        fault: FaultPoint,
    ) -> impl Fn(&Path) -> Result<Box<dyn RecordWriter>, SessionStoreError> {
        move |path: &Path| {
            let inner = CommandRecordStore::open(path)?;
            Ok(Box::new(FaultyStore::new(inner, fault)) as Box<dyn RecordWriter>)
        }
    }

    /// Captures every successful pending/final write for transition-identity assertions.
    struct ObservingRecordWriter {
        inner: CommandRecordStore,
        captured: std::sync::Mutex<Vec<RuntimeCommandRecord>>,
    }

    impl ObservingRecordWriter {
        fn new(inner: CommandRecordStore) -> Self {
            Self {
                inner,
                captured: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn snapshots(&self) -> Vec<RuntimeCommandRecord> {
            self.captured.lock().unwrap().clone()
        }
    }

    impl RecordWriter for ObservingRecordWriter {
        fn acquire_lock(&self, session_id: &str) -> Result<SessionLock, SessionStoreError> {
            self.inner.acquire_lock(session_id)
        }

        fn write_pending(
            &self,
            record: &RuntimeCommandRecord,
            lock: &SessionLock,
        ) -> Result<(), SessionStoreError> {
            self.inner.write_pending(record, lock)?;
            self.captured.lock().unwrap().push(record.clone());
            Ok(())
        }

        fn write_final(
            &self,
            record: &RuntimeCommandRecord,
            lock: &SessionLock,
        ) -> Result<(), SessionStoreError> {
            self.inner.write_final(record, lock)?;
            self.captured.lock().unwrap().push(record.clone());
            Ok(())
        }

        fn write_terminal_unlocked(
            &self,
            record: &RuntimeCommandRecord,
        ) -> Result<(), SessionStoreError> {
            self.inner.write_terminal_unlocked(record)?;
            self.captured.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    fn observing_factory(
        sink: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<ObservingRecordWriter>>>>,
    ) -> impl Fn(&Path) -> Result<Box<dyn RecordWriter>, SessionStoreError> {
        move |path: &Path| {
            let inner = CommandRecordStore::open(path)?;
            let observer = std::sync::Arc::new(ObservingRecordWriter::new(inner));
            *sink.lock().unwrap() = Some(observer.clone());
            Ok(Box::new(ObserverBox(observer)) as Box<dyn RecordWriter>)
        }
    }

    struct ObserverBox(std::sync::Arc<ObservingRecordWriter>);

    impl RecordWriter for ObserverBox {
        fn acquire_lock(&self, session_id: &str) -> Result<SessionLock, SessionStoreError> {
            self.0.acquire_lock(session_id)
        }
        fn write_pending(
            &self,
            record: &RuntimeCommandRecord,
            lock: &SessionLock,
        ) -> Result<(), SessionStoreError> {
            self.0.write_pending(record, lock)
        }
        fn write_final(
            &self,
            record: &RuntimeCommandRecord,
            lock: &SessionLock,
        ) -> Result<(), SessionStoreError> {
            self.0.write_final(record, lock)
        }
        fn write_terminal_unlocked(
            &self,
            record: &RuntimeCommandRecord,
        ) -> Result<(), SessionStoreError> {
            self.0.write_terminal_unlocked(record)
        }
    }

    fn assert_immutable_fields_byte_identical(a: &RuntimeCommandRecord, b: &RuntimeCommandRecord) {
        assert_eq!(a.started_at, b.started_at);
        assert_eq!(a.session_id, b.session_id);
        assert_eq!(a.plan_digest, b.plan_digest);
        assert_eq!(a.profile_id, b.profile_id);
        assert_eq!(
            serde_json::to_vec(&a.request).unwrap(),
            serde_json::to_vec(&b.request).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&a.policy_decision).unwrap(),
            serde_json::to_vec(&b.policy_decision).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&a.source_fingerprints).unwrap(),
            serde_json::to_vec(&b.source_fingerprints).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&a.runtime_context).unwrap(),
            serde_json::to_vec(&b.runtime_context).unwrap()
        );
        assert_eq!(a.parent_session_id, b.parent_session_id);
        assert_eq!(a.work_session_id, b.work_session_id);
        assert_eq!(a.actor, b.actor);
        assert_eq!(a.schema_version, b.schema_version);
        assert_eq!(a.record_kind, b.record_kind);
    }

    fn assert_state_io_error(result: &Result<u8, ControllerError>) {
        match result {
            Err(ControllerError::StateIo { .. }) => {}
            other => panic!("expected ControllerError::StateIo, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn deny_terminal_install_failure_surfaces_state_error_with_zero_executor_calls() {
        let fx = Fixture::new();
        let path = fx.registry.join("sources/descriptors/demo.descriptor.toml");
        let text = fs::read_to_string(&path).unwrap().replace(
            r#"program = "moon"
args = ["run", "demo:build"]
cwd = "demo"
env_keys = ["PATH"]
backend = "moon"
adapter = "moon-task""#,
            r#"program = "rm"
args = ["bin/foo"]
cwd = "demo"
env_keys = ["PATH"]
backend = "native"
adapter = "direct""#,
        );
        fs::write(&path, text).unwrap();
        fx.write_hit_units();

        let spy = SpyExecutor::default();
        let factory = faulty_factory(FaultPoint::TerminalUnlocked);
        let result = run_with_executor_and_store("demo", true, None, &spy, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
        assert!(
            fx.load_state_home_records().is_empty(),
            "no false denied audit record may be installed on state-write failure"
        );
    }

    #[tokio::test]
    async fn gate_terminal_install_failure_surfaces_state_error_with_zero_executor_calls() {
        let mut fx = Fixture::new();
        fx.patch_demo_gated();
        let spy = SpyExecutor::default();
        let factory = faulty_factory(FaultPoint::TerminalUnlocked);
        let result = run_with_executor_and_store("demo", true, None, &spy, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
        assert!(
            fx.load_state_home_records().is_empty(),
            "no false gated audit record may be installed on state-write failure"
        );
    }

    #[tokio::test]
    async fn plan_only_terminal_install_failure_surfaces_state_error_with_zero_executor_calls() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let factory = faulty_factory(FaultPoint::TerminalUnlocked);
        let result = run_with_executor_and_store("demo", false, None, &spy, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
        assert!(
            fx.load_state_home_records().is_empty(),
            "no false planned audit record may be installed on state-write failure"
        );
    }

    #[tokio::test]
    async fn unavailable_class_terminal_install_failure_surfaces_state_error_with_zero_executor_calls()
     {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let factory = faulty_factory(FaultPoint::TerminalUnlocked);
        let result =
            run_with_executor_and_store("interactive-demo", true, None, &spy, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
        assert!(
            fx.load_state_home_records().is_empty(),
            "no false execution_unavailable audit record may be installed on state-write failure"
        );
    }

    #[tokio::test]
    async fn allow_execute_initial_pending_failure_surfaces_state_error_before_executor_call() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let factory = faulty_factory(FaultPoint::InitialPending);
        let result = run_with_executor_and_store("demo", true, None, &spy, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(
            spy.calls(),
            0,
            "executor must not run before the initial pending record is durable"
        );
        assert!(
            fx.load_state_home_records().is_empty(),
            "marker record absent"
        );
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn allow_execute_pid_bearing_pending_replace_failure_surfaces_state_error() {
        let fx = Fixture::new();
        let executor = ScriptedExecutor::new(scripted_spawned_report("completed", 4242));
        let factory = faulty_factory(FaultPoint::FirstFinal);
        let result = run_with_executor_and_store("demo", true, None, &executor, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(executor.calls(), 1);
        let records = fx.load_state_home_records();
        assert_eq!(
            records.len(),
            1,
            "the initial pending record must remain recoverable"
        );
        let rec = &records[0];
        assert_eq!(rec["execution"]["outcome"], "pending");
        assert!(
            rec["execution"]["pid"].is_null(),
            "PID must not appear unless the PID-bearing write actually succeeded"
        );
    }

    #[tokio::test]
    async fn allow_execute_terminal_replace_failure_surfaces_state_error() {
        let fx = Fixture::new();
        let executor = ScriptedExecutor::new(scripted_spawned_report("completed", 4242));
        let factory = faulty_factory(FaultPoint::SecondFinal);
        let result = run_with_executor_and_store("demo", true, None, &executor, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(executor.calls(), 1);
        let records = fx.load_state_home_records();
        assert_eq!(
            records.len(),
            1,
            "the PID-bearing pending record must remain"
        );
        let rec = &records[0];
        assert_eq!(rec["execution"]["outcome"], "pending");
        assert_eq!(rec["execution"]["pid"], 4242);
        assert_eq!(rec["execution"]["started"], true);
    }

    #[tokio::test]
    async fn unavailable_executor_terminal_replace_failure_surfaces_state_error_not_discarded() {
        let fx = Fixture::new();
        let executor = RecordingUnavailableExecutor::default();
        let factory = faulty_factory(FaultPoint::FirstFinal);
        let result = run_with_executor_and_store("demo", true, None, &executor, &factory).await;
        assert_state_io_error(&result);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        let records = fx.load_state_home_records();
        assert_eq!(
            records.len(),
            1,
            "the initial pending record must remain recoverable"
        );
        assert_eq!(records[0]["execution"]["outcome"], "pending");
    }

    #[tokio::test]
    async fn allow_execute_invokes_spy_once() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let code = run("demo", true, None, &spy).await.expect("lifecycle");
        assert_eq!(code, NOT_IMPLEMENTED);
        assert_eq!(spy.calls(), 1);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn plan_only_never_invokes_spy() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let code = run("demo", false, None, &spy).await.expect("lifecycle");
        assert_eq!(code, SUCCESS);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn gate_with_execute_never_invokes_spy() {
        let mut fx = Fixture::new();
        fx.patch_demo_gated();
        let spy = SpyExecutor::default();
        let code = run("demo", true, None, &spy).await.expect("lifecycle");
        assert_eq!(code, POLICY_GATE);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn request_gate_and_deny_never_invoke_spy() {
        for (effect, expected) in [("gate", POLICY_GATE), ("block", POLICY_DENY)] {
            let fx = Fixture::new();
            fx.patch_request_policy(effect);
            let spy = SpyExecutor::default();
            let code = run("demo", true, None, &spy).await.expect("lifecycle");
            assert_eq!(code, expected, "effect={effect}");
            assert_eq!(spy.calls(), 0, "effect={effect}");
            fx.assert_marker_untouched();
        }
    }

    #[tokio::test]
    async fn deny_with_execute_never_invokes_spy() {
        let fx = Fixture::new();
        let path = fx.registry.join("sources/descriptors/demo.descriptor.toml");
        let text = fs::read_to_string(&path).unwrap().replace(
            r#"program = "moon"
args = ["run", "demo:build"]
cwd = "demo"
env_keys = ["PATH"]
backend = "moon"
adapter = "moon-task""#,
            r#"program = "rm"
args = ["bin/foo"]
cwd = "demo"
env_keys = ["PATH"]
backend = "native"
adapter = "direct""#,
        );
        fs::write(&path, text).unwrap();
        fx.write_hit_units();
        write_marker_exe(&fx.path_dir.join("rm"), &fx.marker);

        let spy = SpyExecutor::default();
        let code = run("demo", true, None, &spy).await.expect("lifecycle");
        assert_eq!(code, POLICY_DENY);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn resolution_and_policy_contract_failures_never_invoke_spy() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let code = run("missing-unit", true, None, &spy)
            .await
            .expect("resolution envelope");
        assert_eq!(code, RESOLUTION);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();

        fx.patch_policy_contract_invalid();
        let code = run("demo", true, None, &spy)
            .await
            .expect("contract envelope");
        assert_eq!(code, CONTRACT);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn execution_class_unavailable_never_invokes_spy() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let code = run("interactive-demo", true, None, &spy)
            .await
            .expect("class envelope");
        assert_eq!(code, NOT_IMPLEMENTED);
        assert_eq!(spy.calls(), 0);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn unavailable_executor_is_invoked_once_after_dual_allow() {
        let fx = Fixture::new();
        let executor = RecordingUnavailableExecutor::default();
        let code = run_with_executor("demo", true, None, &executor)
            .await
            .expect("execution-unavailable envelope");
        assert_eq!(code, NOT_IMPLEMENTED);
        assert_eq!(executor.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        fx.assert_marker_untouched();
    }

    /// Minimal packages/ontarch layout for projection store-fault injection.
    struct BinFaultFixture {
        _temp: tempfile::TempDir,
        fixture_root: PathBuf,
        workspace: PathBuf,
        _registry: PathBuf,
        state_home: PathBuf,
        child_spawn_marker: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl BinFaultFixture {
        fn new() -> Self {
            let guard = env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let temp = tempfile::tempdir().unwrap();
            let workspace = temp.path().join("ws");
            let ontarch = workspace.join("packages/ontarch");
            let registry = ontarch.join("registry");
            let state_home = temp.path().join("state");
            fs::create_dir_all(&registry).unwrap();
            fs::create_dir_all(&state_home).unwrap();
            let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/resolution/registry");
            copy_tree(&fixture, &registry);
            // Required sources
            let write_exe = |path: &Path| {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(path).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(path, perms).unwrap();
            };
            let child_spawn_marker = temp.path().join("CHILD_SPAWN_MARKER");
            // Direct spawn evidence: child appends a line if execute_projection reaches spawn.
            {
                use std::os::unix::fs::PermissionsExt;
                let marker_q = child_spawn_marker.to_string_lossy().replace('\'', "'\\''");
                let body = format!("#!/bin/sh\necho ran >> '{marker_q}'\nexit 0\n");
                let ontarch_bin = ontarch.join("bin/ontarch");
                fs::create_dir_all(ontarch_bin.parent().unwrap()).unwrap();
                fs::write(&ontarch_bin, body.as_bytes()).unwrap();
                let mut perms = fs::metadata(&ontarch_bin).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&ontarch_bin, perms).unwrap();
            }
            write_exe(&ontarch.join("bin/ontarch-bin-report"));
            write_exe(&ontarch.join("bin/ontarch-bin-cleanup"));
            fs::create_dir_all(ontarch.join("lib")).unwrap();
            fs::write(ontarch.join("lib/common.sh"), b"#\n").unwrap();
            fs::write(ontarch.join("lib/registry.sh"), b"#\n").unwrap();
            fs::create_dir_all(ontarch.join("policies")).unwrap();
            fs::write(ontarch.join("policies/takogami.agent.policy.toml"), b"#\n").unwrap();
            fs::write(ontarch.join("policies/agent-bin.policy.toml"), b"#\n").unwrap();
            fs::create_dir_all(ontarch.join("schemas")).unwrap();
            fs::write(ontarch.join("schemas/bin-inventory.schema.json"), b"{}\n").unwrap();
            fs::write(
                ontarch.join("schemas/bin-cleanup-plan.schema.json"),
                b"{}\n",
            )
            .unwrap();
            unsafe {
                std::env::set_var("TAKOGAMI_ONTARCH_REGISTRY", registry.display().to_string());
                std::env::set_var("TAKOGAMI_WORKSPACE_ROOT", workspace.display().to_string());
                std::env::set_var("TAKOGAMI_STATE_HOME", state_home.display().to_string());
                std::env::remove_var("TAKOGAMI_PROFILE");
            }
            let fixture_root = temp.path().to_path_buf();
            Self {
                _temp: temp,
                fixture_root,
                workspace,
                _registry: registry,
                state_home,
                child_spawn_marker,
                _guard: guard,
            }
        }

        fn child_spawn_count(&self) -> usize {
            if !self.child_spawn_marker.exists() {
                return 0;
            }
            fs::read_to_string(&self.child_spawn_marker)
                .unwrap()
                .lines()
                .filter(|l| !l.is_empty())
                .count()
        }

        fn load_records(&self) -> Vec<serde_json::Value> {
            let mut out = Vec::new();
            if !self.state_home.exists() {
                return out;
            }
            for entry in fs::read_dir(&self.state_home).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || !name.ends_with(".json") {
                    continue;
                }
                out.push(serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap());
            }
            out
        }
    }

    async fn run_bin_with(
        sub: BinCommand,
        executor: &dyn ProjectionExecutor,
        open_store: &StoreFactory,
        state_home: &Path,
    ) -> Result<u8, ControllerError> {
        let sink = OutputSink {
            json: true,
            no_color: true,
        };
        run_bin(&sink, &sub, None, Some(state_home), executor, open_store).await
    }

    #[tokio::test]
    async fn projection_initial_pending_failure_zero_executor_calls() {
        let fx = BinFaultFixture::new();
        let spy = SpyProjectionExecutor::default();
        let factory = faulty_factory(FaultPoint::InitialPending);
        let result = run_bin_with(BinCommand::Report, &spy, &factory, &fx.state_home).await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
    }

    #[tokio::test]
    async fn projection_gate_terminal_failure_zero_spawn() {
        let fx = BinFaultFixture::new();
        let spy = SpyProjectionExecutor::default();
        let factory = faulty_factory(FaultPoint::TerminalUnlocked);
        let result = run_bin_with(
            BinCommand::Cleanup {
                mode: BinCleanupMode::DryRun,
                scope: None,
            },
            &spy,
            &factory,
            &fx.state_home,
        )
        .await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
    }

    #[tokio::test]
    async fn projection_deny_terminal_failure_zero_spawn() {
        let fx = BinFaultFixture::new();
        let spy = SpyProjectionExecutor::default();
        let factory = faulty_factory(FaultPoint::TerminalUnlocked);
        let result = run_bin_with(
            BinCommand::Cleanup {
                mode: BinCleanupMode::Archive,
                scope: None,
            },
            &spy,
            &factory,
            &fx.state_home,
        )
        .await;
        assert_state_io_error(&result);
        assert_eq!(spy.calls(), 0);
    }

    #[tokio::test]
    async fn projection_transition_immutable_fields_are_byte_identical() {
        let fx = BinFaultFixture::new();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(None));
        let factory = observing_factory(sink.clone());
        let exe = FakeSpawnProjectionExecutor::default();
        let result = run_bin_with(BinCommand::Report, &exe, &factory, &fx.state_home).await;
        // Fake spawn yields invalid payload -> controller_error terminal, but transitions exist.
        assert!(result.is_ok());
        let observer = sink.lock().unwrap().clone().expect("observer installed");
        let snaps = observer.snapshots();
        assert!(
            snaps.len() >= 2,
            "expected pending and at least one final, got {}",
            snaps.len()
        );
        let pending = &snaps[0];
        assert_eq!(pending.execution.outcome, "pending");
        for later in &snaps[1..] {
            assert_immutable_fields_byte_identical(pending, later);
        }
        assert_eq!(exe.calls.load(Ordering::SeqCst), 1);
    }

    fn write_exe_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    fn write_split_projection_helpers(early: &Path, late: &Path) {
        fs::create_dir_all(early).unwrap();
        fs::create_dir_all(late).unwrap();
        let names = [
            "bash", "jq", "dirname", "basename", "readlink", "date", "du", "awk", "wc", "tr",
            "stat", "mkdir", "mktemp", "rm", "cat", "cp", "mv", "grep", "sed", "head", "fd",
            "find",
        ];
        for name in names {
            write_exe_file(&late.join(name));
        }
        write_exe_file(&early.join("bash"));
    }

    /// Counts real `execute_projection` entries while delegating to the production executor.
    struct CountingProjectionExecutor<E> {
        inner: E,
        calls: AtomicU32,
    }

    impl<E> CountingProjectionExecutor<E> {
        fn new(inner: E) -> Self {
            Self {
                inner,
                calls: AtomicU32::new(0),
            }
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl<E: ProjectionExecutor> ProjectionExecutor for CountingProjectionExecutor<E> {
        async fn execute_projection(
            &self,
            plan: &crate::policy::AuthorizedProjectionPlan,
            options: &crate::execution::ExecutionOptions,
        ) -> ExecutionReport {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.execute_projection(plan, options).await
        }
    }

    #[derive(Clone, Copy)]
    enum HelperShadowKind {
        DistinctBytes,
        SameBytes,
        WorldWritable,
        SymlinkToWorldWritableTarget,
    }

    async fn run_bin_with_helper_shadow(
        fixture_root: &Path,
        early: &Path,
        late: &Path,
        state_home: &Path,
        kind: HelperShadowKind,
        outside: Option<&Path>,
    ) -> (
        Result<u8, ControllerError>,
        std::sync::Arc<ObservingRecordWriter>,
        u32,
    ) {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(None));
        let factory = observing_factory(sink.clone());
        let late_jq = late.join("jq");
        let shadow = match kind {
            HelperShadowKind::DistinctBytes => HelperShadowSpec::DistinctBytes,
            HelperShadowKind::SameBytes => HelperShadowSpec::SameBytesAs(late_jq),
            HelperShadowKind::WorldWritable => HelperShadowSpec::WorldWritable,
            HelperShadowKind::SymlinkToWorldWritableTarget => {
                let outside = outside.expect("outside dir required for symlink shadow");
                fs::create_dir_all(outside).unwrap();
                let target = outside.join("jq-ww");
                write_exe_file(&target);
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&target).unwrap().permissions();
                perms.set_mode(0o757);
                fs::set_permissions(&target, perms).unwrap();
                HelperShadowSpec::SymlinkTo(target)
            }
        };
        let mutation = ProjectionTestMutation::new(
            fixture_root.to_path_buf(),
            MutationKind::InsertHelperShadow {
                dir: early.to_path_buf(),
                name: "jq".into(),
                shadow,
            },
        );
        let exe = CountingProjectionExecutor::new(MutatingProjectionExecutor::new(mutation));
        crate::projection::install_test_search_dirs(vec![early.to_path_buf(), late.to_path_buf()]);
        let result = run_bin_with(BinCommand::Report, &exe, &factory, state_home).await;
        crate::projection::clear_test_search_dirs();
        let observer = sink.lock().unwrap().clone().expect("observer installed");
        let calls = exe.calls();
        (result, observer, calls)
    }

    async fn run_bin_with_mutation(
        fx: &BinFaultFixture,
        kind: MutationKind,
    ) -> (
        Result<u8, ControllerError>,
        std::sync::Arc<ObservingRecordWriter>,
        u32,
    ) {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(None));
        let factory = observing_factory(sink.clone());
        let mutation = ProjectionTestMutation::new(fx.fixture_root.clone(), kind);
        let exe = CountingProjectionExecutor::new(MutatingProjectionExecutor::new(mutation));
        let result = run_bin_with(BinCommand::Report, &exe, &factory, &fx.state_home).await;
        let observer = sink.lock().unwrap().clone().expect("observer installed");
        (result, observer, exe.calls())
    }

    fn assert_hooked_preflight_terminal(
        result: &Result<u8, ControllerError>,
        observer: &ObservingRecordWriter,
        calls: u32,
        child_spawns: usize,
        workspace: &Path,
        err_code: &str,
    ) {
        assert_eq!(
            result.as_ref().unwrap().clone(),
            crate::exit_codes::EXECUTION_IO
        );
        assert_eq!(calls, 1);
        assert_eq!(child_spawns, 0);
        let snaps = observer.snapshots();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].execution.outcome, "pending");
        assert!(snaps[0].execution.pid.is_none());
        let terminal = &snaps[1];
        assert_eq!(terminal.execution.outcome, "failed_to_spawn");
        assert!(!terminal.execution.started);
        assert!(terminal.execution.pid.is_none());
        assert!(terminal.execution.exit_code.is_none());
        let err = terminal.error.as_ref().expect("terminal error required");
        assert_eq!(err.code, err_code);
        assert_immutable_fields_byte_identical(&snaps[0], terminal);
        assert!(!err.message.contains(workspace.to_string_lossy().as_ref()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_executable_removal_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::RemoveExecutable).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_cwd_rename_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) = run_bin_with_mutation(&fx, MutationKind::RenameCwd).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_source_drift_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::RewriteSource).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_source_removal_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::RemoveSource).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_source_symlink_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::SourceSymlink).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_source_dangling_symlink_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::SourceDanglingSymlink).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_source_fifo_fails_without_blocking_no_spawn() {
        let fx = BinFaultFixture::new();
        let start = std::time::Instant::now();
        let (result, observer, calls) = run_bin_with_mutation(&fx, MutationKind::SourceFifo).await;
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "FIFO mutation must not block"
        );
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_source_directory_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::SourceDirectory).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_same_length_source_drift_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::SourceSameLengthDrift).await;
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hooked_helper_content_drift_fails_preflight_no_spawn() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        crate::projection::install_test_search_dirs(vec![early.clone(), late.clone()]);
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::RewriteHelper { name: "jq".into() }).await;
        crate::projection::clear_test_search_dirs();
        assert_hooked_preflight_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &fx.workspace,
            "projection_contract_changed",
        );
    }

    fn assert_untrusted_helper_terminal(
        result: &Result<u8, ControllerError>,
        observer: &ObservingRecordWriter,
        calls: u32,
        child_spawns: usize,
        early: &Path,
        late: &Path,
    ) {
        assert_eq!(
            result.as_ref().unwrap().clone(),
            crate::exit_codes::EXECUTION_IO
        );
        assert_eq!(
            calls, 1,
            "execute_projection must be entered exactly once after authorization"
        );
        assert_eq!(child_spawns, 0, "child spawn count must remain zero");
        let snaps = observer.snapshots();
        assert_eq!(snaps.len(), 2, "pending then terminal only (no PID)");
        assert_eq!(snaps[0].execution.outcome, "pending");
        assert!(snaps[0].execution.pid.is_none());
        let terminal = &snaps[1];
        assert_eq!(terminal.execution.outcome, "failed_to_spawn");
        assert!(!terminal.execution.started);
        assert!(terminal.execution.pid.is_none());
        assert!(terminal.execution.exit_code.is_none());
        let err = terminal.error.as_ref().expect("terminal error required");
        assert_eq!(err.code, "projection_contract_changed");
        assert_immutable_fields_byte_identical(&snaps[0], terminal);
        assert!(!err.message.contains(early.to_string_lossy().as_ref()));
        assert!(!err.message.contains(late.to_string_lossy().as_ref()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_helper_shadow_is_terminal_no_spawn() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::DistinctBytes,
            None,
        )
        .await;
        assert_untrusted_helper_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &early,
            &late,
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_same_byte_helper_shadow_is_terminal_no_spawn() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::SameBytes,
            None,
        )
        .await;
        assert_untrusted_helper_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &early,
            &late,
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_untrusted_helper_first_match_is_terminal_no_spawn() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::WorldWritable,
            None,
        )
        .await;
        assert_untrusted_helper_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &early,
            &late,
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_untrusted_helper_executor_called_exactly_once() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::WorldWritable,
            None,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(calls, 1);
        assert_eq!(observer.snapshots().len(), 2);
        assert_eq!(fx.child_spawn_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_untrusted_helper_child_spawn_count_zero() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (_result, _observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::WorldWritable,
            None,
        )
        .await;
        assert_eq!(calls, 1);
        assert_eq!(fx.child_spawn_count(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_untrusted_helper_preserves_record_identity() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::WorldWritable,
            None,
        )
        .await;
        assert_untrusted_helper_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &early,
            &late,
        );
        let snaps = observer.snapshots();
        assert_immutable_fields_byte_identical(&snaps[0], &snaps[1]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_world_writable_helper_shadow_is_terminal_no_spawn() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::WorldWritable,
            None,
        )
        .await;
        assert_untrusted_helper_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &early,
            &late,
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_authorization_world_writable_symlink_target_is_terminal_no_spawn() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        let outside = fx.workspace.join("helper-outside");
        write_split_projection_helpers(&early, &late);
        let (result, observer, calls) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::SymlinkToWorldWritableTarget,
            Some(&outside),
        )
        .await;
        assert_untrusted_helper_terminal(
            &result,
            &observer,
            calls,
            fx.child_spawn_count(),
            &early,
            &late,
        );
        let snaps = observer.snapshots();
        let msg = &snaps[1].error.as_ref().unwrap().message;
        assert!(!msg.contains(outside.to_string_lossy().as_ref()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn helper_shadow_diagnostic_omits_absolute_roots() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, _) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::DistinctBytes,
            None,
        )
        .await;
        assert_eq!(result.unwrap(), crate::exit_codes::EXECUTION_IO);
        let terminal = &observer.snapshots()[1];
        let msg = &terminal.error.as_ref().unwrap().message;
        assert!(!msg.contains(early.to_string_lossy().as_ref()));
        assert!(!msg.contains(late.to_string_lossy().as_ref()));
        assert!(!msg.contains("/usr/bin"));
        assert!(!msg.contains("/opt/homebrew"));
        let listed = fx.load_records();
        let rec = &listed[0];
        let dumped = serde_json::to_string(rec).unwrap();
        assert!(!dumped.contains(early.to_string_lossy().as_ref()));
        assert!(!dumped.contains(late.to_string_lossy().as_ref()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn world_writable_helper_shadow_diagnostic_omits_absolute_roots() {
        let fx = BinFaultFixture::new();
        let early = fx.workspace.join("helper-early");
        let late = fx.workspace.join("helper-late");
        write_split_projection_helpers(&early, &late);
        let (result, observer, _) = run_bin_with_helper_shadow(
            &fx.fixture_root,
            &early,
            &late,
            &fx.state_home,
            HelperShadowKind::WorldWritable,
            None,
        )
        .await;
        assert_eq!(result.unwrap(), crate::exit_codes::EXECUTION_IO);
        let snaps = observer.snapshots();
        let msg = &snaps[1].error.as_ref().unwrap().message;
        assert!(!msg.contains(early.to_string_lossy().as_ref()));
        assert!(!msg.contains(late.to_string_lossy().as_ref()));
        assert!(!msg.contains("/usr/bin"));
        assert!(!msg.contains("/opt/homebrew"));
    }

    #[tokio::test]
    async fn projection_preflight_failure_observer_requires_safe_terminal() {
        let fx = BinFaultFixture::new();
        let (result, observer, calls) =
            run_bin_with_mutation(&fx, MutationKind::RemoveSource).await;
        assert_eq!(calls, 1);
        assert_eq!(result.unwrap(), crate::exit_codes::EXECUTION_IO);
        let snaps = observer.snapshots();
        assert_eq!(snaps.len(), 2, "pending then terminal only (no PID)");
        assert_eq!(snaps[0].execution.outcome, "pending");
        let terminal = &snaps[1];
        assert_eq!(terminal.execution.outcome, "failed_to_spawn");
        assert!(!terminal.execution.started);
        assert!(terminal.execution.pid.is_none());
        let err = terminal.error.as_ref().expect("terminal error required");
        assert_eq!(err.code, "projection_contract_changed");
        assert!(
            !err.message
                .contains(fx.workspace.to_string_lossy().as_ref())
        );
        assert_immutable_fields_byte_identical(&snaps[0], terminal);
    }

    /// Reports a successful spawn without running a child (store-transition matrix).
    struct FakeSpawnProjectionExecutor {
        calls: AtomicU32,
    }

    impl Default for FakeSpawnProjectionExecutor {
        fn default() -> Self {
            Self {
                calls: AtomicU32::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProjectionExecutor for FakeSpawnProjectionExecutor {
        async fn execute_projection(
            &self,
            _plan: &crate::policy::AuthorizedProjectionPlan,
            _options: &crate::execution::ExecutionOptions,
        ) -> ExecutionReport {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut report = ExecutionReport::idle("completed");
            report.spawned = true;
            report.pid = Some(4242);
            report.exit_code = Some(0);
            report.outcome = "completed".into();
            report.stdout.encoding = "utf-8".into();
            report.stderr.encoding = "utf-8".into();
            // Invalid payload so terminal path still finalizes with controller_error.
            report.stdout.bytes = b"not-json".to_vec();
            report.stdout.total_bytes = 8;
            report
        }
    }

    #[tokio::test]
    async fn projection_pid_bearing_pending_failure_child_attempted() {
        let fx = BinFaultFixture::new();
        let exe = FakeSpawnProjectionExecutor::default();
        let factory = faulty_factory(FaultPoint::FirstFinal);
        let result = run_bin_with(BinCommand::Report, &exe, &factory, &fx.state_home).await;
        assert_state_io_error(&result);
        assert_eq!(exe.calls.load(Ordering::SeqCst), 1);
        let listed = fx.load_records();
        assert!(
            listed
                .iter()
                .any(|r| r["execution"]["outcome"] == "pending"),
            "initial pending must remain after PID-bearing write fault"
        );
    }

    #[tokio::test]
    async fn projection_terminal_failure_pid_bearing_remains() {
        let fx = BinFaultFixture::new();
        let exe = FakeSpawnProjectionExecutor::default();
        let factory = faulty_factory(FaultPoint::SecondFinal);
        let result = run_bin_with(BinCommand::Report, &exe, &factory, &fx.state_home).await;
        assert_state_io_error(&result);
        assert_eq!(exe.calls.load(Ordering::SeqCst), 1);
        let listed = fx.load_records();
        assert!(
            listed
                .iter()
                .any(|r| { r["execution"]["started"] == true && r["execution"]["pid"] == 4242 }),
            "PID-bearing pending must remain after terminal write fault: {listed:?}"
        );
    }
}

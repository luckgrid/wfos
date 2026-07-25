//! Command handlers for discovery / query / doctor and lifecycle planning.

use std::path::{Path, PathBuf};

use crate::cli::{Command, ListTarget, SessionCommand};
use crate::contracts::types::{
    ExecutionRecord, OutputSummary, RECORD_KIND_COMMAND_EXECUTION, RequestRecord,
    RuntimeCommandRecord, SCHEMA_VERSION,
};
use crate::contracts::{
    ExecutionClass, PolicyDecision, StateHomeInputs, ensure_state_home, resolve_session_state_home,
};
use crate::doctor::{self, DoctorInputs};
use crate::error::{ControllerError, ExecutionDeferredDetails, PolicyOutcomeDetails};
use crate::execution::{ExecutionMode, ExecutionOptions, Executor, TokioExecutor};
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
        _ => Err(ControllerError::internal(
            "dispatch_implemented called for unimplemented command",
        )),
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
            // S6.1-08: surface the same skipped-record diagnostics `list` reports, instead of
            // discarding them.
            let (record, diagnostics) =
                show_latest_with_diagnostics(&store).map_err(|e| match e {
                    crate::sessions::SessionStoreError::NotFound(_) => {
                        ControllerError::not_found("no command execution records")
                    }
                    other => ControllerError::StateIo {
                        message: other.to_string(),
                        code: other.code().into(),
                    },
                })?;
            emit_session_record(sink, &record, &diagnostics.skipped)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{RegistryGeneration, fingerprint_file};
    use crate::execution::{ExecutionReport, SpyExecutor, UnavailableExecutor};
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
}

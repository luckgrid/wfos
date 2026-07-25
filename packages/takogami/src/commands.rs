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
    ExternalAdapters, Freshness, ProcessAdapters, RefreshKind, RegistryAccess, discover_from_scan,
    filter_tools, filter_units, find_unit, parse_filters, resolve_registry_paths,
};
use crate::resolution::{CorrelationIdGenerator, DefaultIdGenerator, ResolutionRequest, resolve};
use crate::sessions::{
    CommandRecordStore, RuntimeContextEnv, collect_runtime_context, list_sessions, show_latest,
    show_session, utc_now_rfc3339,
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
            )
            .await
        }
        _ => Err(ControllerError::internal(
            "dispatch_implemented called for unimplemented command",
        )),
    }
}

/// Internal coordinator accepting an injected executor (spy in tests; Tokio in production).
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
) -> Result<u8, ControllerError> {
    let access = access()?;
    let mut id_gen = DefaultIdGenerator::default();
    let session_id = id_gen.next_id();
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let env_profile = std::env::var("TAKOGAMI_PROFILE").ok();

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
            let record = base_record_from_rejected(&rejected, verb.as_str(), execute, outcome);
            let _ = persist_terminal(&state_home, &record);
            match rejected.decision() {
                PolicyDecision::Deny { reason, .. } => {
                    let err = ControllerError::PolicyDeny {
                        reason: reason.clone(),
                        details: Box::new(PolicyOutcomeDetails::from_rejected(&rejected)),
                    };
                    return sink
                        .emit_policy_outcome(verb.as_str(), &err, &rejected, success.freshness)
                        .map_err(|e| ControllerError::internal(e.to_string()));
                }
                PolicyDecision::Gate { reason, .. } => {
                    let err = ControllerError::PolicyGate {
                        reason: reason.clone(),
                        details: Box::new(PolicyOutcomeDetails::from_rejected(&rejected)),
                    };
                    return sink
                        .emit_policy_outcome(verb.as_str(), &err, &rejected, success.freshness)
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
        );
        let _ = persist_terminal(&state_home, &record);
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
            )
            .map_err(|e| ControllerError::internal(e.to_string()));
    }

    if execute {
        let store =
            CommandRecordStore::open(&state_home).map_err(|e| ControllerError::StateIo {
                message: e.to_string(),
                code: e.code().into(),
            })?;
        let lock = store
            .acquire_lock(&session_id)
            .map_err(|e| ControllerError::StateIo {
                message: e.to_string(),
                code: e.code().into(),
            })?;

        let mut pending =
            base_record_from_authorized(&authorized, verb.as_str(), true, "pending", true);
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
            let _ = store.write_final(&pending, &lock);
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
            let _ = store.write_final(&final_rec, &lock);
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
            )
            .map_err(|e| ControllerError::internal(e.to_string()));
    }

    let record = base_record_from_authorized(&authorized, verb.as_str(), false, "planned", true);
    let _ = persist_terminal(&state_home, &record);

    if explain {
        sink.emit_explanation_with_policy(
            verb.as_str(),
            &authorized,
            &success.explanation,
            success.freshness,
        )
        .map_err(|e| ControllerError::internal(e.to_string()))
    } else {
        sink.emit_plan_with_policy(
            verb.as_str(),
            &authorized,
            &success.explanation,
            success.freshness,
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
    state_home: &Path,
    record: &RuntimeCommandRecord,
) -> Result<(), ControllerError> {
    ensure_state_home(state_home).map_err(|e| ControllerError::StateIo {
        message: e.to_string(),
        code: "state_io".into(),
    })?;
    let store = CommandRecordStore::open(state_home).map_err(|e| ControllerError::StateIo {
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
) -> RuntimeCommandRecord {
    let plan = authorized.plan();
    let resolved = plan.resolved();
    let (runtime_context, _) = runtime_context_now();
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
        error: None,
    }
}

fn base_record_from_rejected(
    rejected: &crate::policy::RejectedPolicyOutcome,
    command: &str,
    execute: bool,
    outcome: &str,
) -> RuntimeCommandRecord {
    let plan = rejected.plan();
    let resolved = plan.resolved();
    let (runtime_context, _) = runtime_context_now();
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
    let profiles = access.load_profiles().ok();
    let profile_home = profiles.as_ref().and_then(|doc| {
        let id = cli_profile
            .map(str::to_string)
            .or_else(|| std::env::var("TAKOGAMI_PROFILE").ok())
            .or_else(|| {
                doc.profiles
                    .iter()
                    .find(|p| p.id == "workspace-dev")
                    .map(|p| p.id.clone())
            });
        id.and_then(|want| {
            doc.profiles
                .iter()
                .find(|p| p.id == want)
                .and_then(|p| p.session_state_home.clone())
        })
    });
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
            emit_session_record(sink, &record)
        }
        SessionCommand::Latest => {
            let record = show_latest(&store).map_err(|e| match e {
                crate::sessions::SessionStoreError::NotFound(_) => {
                    ControllerError::not_found("no command execution records")
                }
                other => ControllerError::StateIo {
                    message: other.to_string(),
                    code: other.code().into(),
                },
            })?;
            emit_session_record(sink, &record)
        }
    }
}

fn emit_session_record(
    sink: &OutputSink,
    record: &RuntimeCommandRecord,
) -> Result<u8, ControllerError> {
    let data =
        serde_json::to_value(record).map_err(|e| ControllerError::internal(e.to_string()))?;
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
    use crate::execution::{SpyExecutor, UnavailableExecutor};
    use crate::exit_codes::{
        CONTRACT, NOT_IMPLEMENTED, POLICY_DENY, POLICY_GATE, RESOLUTION, SUCCESS,
    };
    use crate::resolution::LifecycleVerb;
    use std::fs;
    use std::path::{Path, PathBuf};
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
            let guard = env_lock().lock().unwrap();
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

    #[tokio::test]
    async fn allow_execute_invokes_spy_once() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let code = run("demo", true, None, &spy).await.expect("lifecycle");
        assert_eq!(code, NOT_IMPLEMENTED);
        assert_eq!(spy.calls.get(), 1);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn plan_only_never_invokes_spy() {
        let fx = Fixture::new();
        let spy = SpyExecutor::default();
        let code = run("demo", false, None, &spy).await.expect("lifecycle");
        assert_eq!(code, SUCCESS);
        assert_eq!(spy.calls.get(), 0);
        fx.assert_marker_untouched();
    }

    #[tokio::test]
    async fn gate_with_execute_never_invokes_spy() {
        let mut fx = Fixture::new();
        fx.patch_demo_gated();
        let spy = SpyExecutor::default();
        let code = run("demo", true, None, &spy).await.expect("lifecycle");
        assert_eq!(code, POLICY_GATE);
        assert_eq!(spy.calls.get(), 0);
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
            assert_eq!(spy.calls.get(), 0, "effect={effect}");
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
        assert_eq!(spy.calls.get(), 0);
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
        assert_eq!(spy.calls.get(), 0);
        fx.assert_marker_untouched();

        fx.patch_policy_contract_invalid();
        let code = run("demo", true, None, &spy)
            .await
            .expect("contract envelope");
        assert_eq!(code, CONTRACT);
        assert_eq!(spy.calls.get(), 0);
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
        assert_eq!(spy.calls.get(), 0);
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

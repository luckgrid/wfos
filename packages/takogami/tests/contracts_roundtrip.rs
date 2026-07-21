//! Contract round-trip and schema validation tests (E09.S2 / S2.1).
//!
//! NOTE: these tests document behavior before relying on later stories.
//! They validate wire contracts only — no discovery, resolution, spawn, or session I/O.

use jsonschema::Validator;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use takogami::contracts::types::{
    RECORD_KIND_COMMAND_EXECUTION, SCHEMA_VERSION, require_schema_version,
};
use takogami::contracts::{
    ChildOutput, CommandEnvelope, EnvelopeMetrics, ExecutionRecord, OutputSummary, PolicyDecision,
    RegistryGeneration, RequestRecord, ResolvedCommand, RuntimeCommandRecord, RuntimeContext,
    SourceFingerprint, StateHomeInputs, fingerprint_bytes, parse_legacy_entrypoint,
    resolve_session_state_home,
};

fn ontarch_schema(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ontarch/schemas")
        .join(name)
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/contracts")
        .join(name)
}

fn load_json(path: &PathBuf) -> Value {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn compile_schema(name: &str) -> Validator {
    let schema = load_json(&ontarch_schema(name));
    Validator::new(&schema).expect("compile schema")
}

fn assert_valid(validator: &Validator, value: &Value) {
    if let Err(error) = validator.validate(value) {
        panic!("schema validation failed: {error}\nvalue={value}");
    }
}

fn assert_invalid(validator: &Validator, value: &Value) {
    assert!(
        validator.validate(value).is_err(),
        "expected invalid value to fail schema: {value}"
    );
}

fn minimal_record(outcome: &str) -> RuntimeCommandRecord {
    RuntimeCommandRecord {
        schema_version: SCHEMA_VERSION.into(),
        record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
        session_id: "s".into(),
        parent_session_id: None,
        work_session_id: None,
        runtime_context: None,
        started_at: "2026-07-19T00:00:00Z".into(),
        ended_at: Some("2026-07-19T00:00:01Z".into()),
        actor: "agent".into(),
        profile_id: "workspace-dev".into(),
        request: RequestRecord {
            command: "build".into(),
            unit_id: Some("takogami".into()),
            verb: Some("build".into()),
            flags: vec![],
        },
        resolution: None,
        policy_decision: PolicyDecision::Allow {
            matched_rules: vec![],
        },
        execution: ExecutionRecord {
            started: false,
            pid: None,
            exit_code: None,
            signal: None,
            outcome: outcome.into(),
        },
        source_fingerprints: vec![],
        output_summary: OutputSummary {
            stdout_bytes: 0,
            stderr_bytes: 0,
            truncated: false,
            encoding: "utf-8".into(),
            compressor: "none".into(),
        },
        validation: None,
        error: None,
    }
}

#[test]
fn command_envelope_fixture_round_trips_and_validates() {
    let validator = compile_schema("command-output.schema.json");
    let value = load_json(&fixture("command-envelope-valid.json"));
    assert_valid(&validator, &value);
    let envelope: CommandEnvelope = serde_json::from_value(value.clone()).expect("deserialize");
    assert_eq!(envelope.schema_version, SCHEMA_VERSION);
    let again = serde_json::to_value(&envelope).expect("serialize");
    assert_eq!(again["schema_version"], "0.1.0");
    assert_eq!(again["command"], "doctor");
    assert_valid(&validator, &again);
}

#[test]
fn command_record_fixture_round_trips_and_validates() {
    let validator = compile_schema("runtime-command-record.schema.json");
    let value = load_json(&fixture("runtime-command-record-valid.json"));
    assert_valid(&validator, &value);
    let record: RuntimeCommandRecord = serde_json::from_value(value.clone()).expect("deserialize");
    assert_eq!(record.record_kind, RECORD_KIND_COMMAND_EXECUTION);
    assert_eq!(record.actor, "agent");
    assert!(record.runtime_context.is_none());
    assert!(matches!(
        record.policy_decision,
        PolicyDecision::Deny { .. }
    ));
    assert_eq!(record.execution.outcome, "denied");
    assert!(record.execution.pid.is_none());
    let again = serde_json::to_value(&record).expect("serialize");
    assert_valid(&validator, &again);
}

#[test]
fn command_record_herdr_and_tmux_runtime_context_validate() {
    let validator = compile_schema("runtime-command-record.schema.json");
    for name in [
        "runtime-command-record-herdr.json",
        "runtime-command-record-tmux.json",
    ] {
        let value = load_json(&fixture(name));
        assert_valid(&validator, &value);
        let record: RuntimeCommandRecord =
            serde_json::from_value(value.clone()).expect("deserialize");
        assert_eq!(record.record_kind, RECORD_KIND_COMMAND_EXECUTION);
        assert!(record.runtime_context.is_some());
        let again = serde_json::to_value(&record).expect("serialize");
        assert_valid(&validator, &again);
    }
}

#[test]
fn command_record_missing_or_wrong_record_kind_fails_schema() {
    let validator = compile_schema("runtime-command-record.schema.json");
    let mut missing = load_json(&fixture("runtime-command-record-valid.json"));
    missing.as_object_mut().unwrap().remove("record_kind");
    assert_invalid(&validator, &missing);

    let mut wrong = load_json(&fixture("runtime-command-record-valid.json"));
    wrong["record_kind"] = Value::String("work_session".into());
    assert_invalid(&validator, &wrong);
}

#[test]
fn command_record_rejects_socket_path_and_scrollback() {
    let validator = compile_schema("runtime-command-record.schema.json");
    let mut with_socket = load_json(&fixture("runtime-command-record-valid.json"));
    with_socket["socket_path"] = Value::String("/tmp/herdr.sock".into());
    assert_invalid(&validator, &with_socket);

    let mut with_scrollback = load_json(&fixture("runtime-command-record-valid.json"));
    with_scrollback["scrollback"] = Value::String("pane output blob".into());
    assert_invalid(&validator, &with_scrollback);

    let mut nested = load_json(&fixture("runtime-command-record-herdr.json"));
    nested["runtime_context"]["socket_path"] = Value::String("/var/run/herdr.sock".into());
    assert_invalid(&validator, &nested);
}

#[test]
fn command_record_schema_omits_sensitive_property_names() {
    let schema = load_json(&ontarch_schema("runtime-command-record.schema.json"));
    let props = schema["properties"].as_object().expect("properties");
    assert!(!props.contains_key("socket_path"));
    assert!(!props.contains_key("scrollback"));
    assert!(!props.contains_key("HERDR_SOCKET_PATH"));
    let ctx_props = schema["properties"]["runtime_context"]["properties"]
        .as_object()
        .expect("runtime_context.properties");
    assert!(!ctx_props.contains_key("socket_path"));
    assert!(!ctx_props.contains_key("scrollback"));
    assert!(ctx_props.contains_key("provider"));
}

#[test]
fn schema_version_mismatch_is_typed_contract_error() {
    let err = require_schema_version("9.9.9").unwrap_err();
    assert!(err.contains("schema_version mismatch"));
    assert!(require_schema_version(SCHEMA_VERSION).is_ok());

    let validator = compile_schema("command-output.schema.json");
    let mut bad = load_json(&fixture("command-envelope-valid.json"));
    bad["schema_version"] = Value::String("9.9.9".into());
    assert_invalid(&validator, &bad);
}

#[test]
fn missing_required_envelope_fields_fail_schema() {
    let validator = compile_schema("command-output.schema.json");
    let bad = serde_json::json!({
        "schema_version": "0.1.0",
        "command": "scan"
    });
    assert_invalid(&validator, &bad);
}

#[test]
fn unknown_envelope_fields_fail_schema() {
    let validator = compile_schema("command-output.schema.json");
    let mut bad = load_json(&fixture("command-envelope-valid.json"));
    bad["secret_env"] = Value::String("should-not-appear".into());
    assert_invalid(&validator, &bad);
}

#[test]
fn policy_decision_variants_round_trip() {
    let allow = PolicyDecision::Allow {
        matched_rules: vec!["panoply.agent:allow-moon".into()],
    };
    let gate = PolicyDecision::Gate {
        policy_id: "agent-bin".into(),
        rule_id: "gate-archive".into(),
        reason: "archive requires approval".into(),
        required_approval: "human".into(),
    };
    let deny = PolicyDecision::Deny {
        policy_id: "agent-git".into(),
        rule_id: "block-push".into(),
        reason: "remote writes blocked".into(),
    };
    for decision in [allow, gate, deny] {
        let value = serde_json::to_value(&decision).unwrap();
        let back: PolicyDecision = serde_json::from_value(value).unwrap();
        assert_eq!(back, decision);
    }
}

#[test]
fn resolved_command_redacts_env_to_keys_only() {
    let cmd = ResolvedCommand {
        schema_version: SCHEMA_VERSION.into(),
        session_id: "s1".into(),
        unit_id: "takogami".into(),
        verb: "build".into(),
        descriptor_path: "packages/ontarch/descriptors/takogami.descriptor.toml".into(),
        descriptor_fingerprint: format!("sha256:{}", fingerprint_bytes(b"x").digest),
        native_manifests: vec!["Cargo.toml".into()],
        backend: "moon".into(),
        adapter: "moon-task".into(),
        program: "moon".into(),
        argv: vec!["run".into(), "takogami:build".into()],
        cwd: "Build/src/workspaces/wfos".into(),
        env_keys: vec!["PATH".into(), "CARGO_HOME".into()],
        profile_id: "workspace-dev".into(),
        policy_ids: vec!["panoply.agent".into()],
        registry_generation: RegistryGeneration {
            generated_at: "2026-07-19T00:00:00Z".into(),
            source_fingerprints: vec![SourceFingerprint {
                path: "takogami.descriptor.toml".into(),
                algorithm: "sha256".into(),
                digest: fingerprint_bytes(b"descriptor").digest,
            }],
        },
    };
    let value = serde_json::to_value(&cmd).unwrap();
    let text = value.to_string();
    assert!(
        value["env_keys"]
            .as_array()
            .unwrap()
            .contains(&Value::String("PATH".into()))
    );
    assert!(!text.contains("SECRET"));
    assert!(!text.contains("password"));
    // No inherited environment dump field.
    assert!(value.get("env").is_none());
    assert!(value.get("environment").is_none());
}

#[test]
fn bounded_child_output_and_terminal_outcomes() {
    let envelope: CommandEnvelope = CommandEnvelope {
        schema_version: SCHEMA_VERSION.into(),
        command: "build".into(),
        session_id: Some("s1".into()),
        status: "ok".into(),
        exit_code: 0,
        data: None,
        explanation: None,
        diagnostics: vec![],
        child: Some(ChildOutput {
            stdout: Some("ok".into()),
            stderr: Some("".into()),
            truncated: true,
            encoding: "utf-8".into(),
        }),
        metrics: Some(EnvelopeMetrics {
            registry_cache: "hit".into(),
            output_bytes: 2,
            compressor: "none".into(),
            gain: None,
        }),
    };
    let validator = compile_schema("command-output.schema.json");
    assert_valid(&validator, &serde_json::to_value(&envelope).unwrap());

    let record_validator = compile_schema("runtime-command-record.schema.json");
    for outcome in [
        "completed",
        "denied",
        "gated",
        "failed_to_spawn",
        "interrupted",
        "controller_error",
        "abandoned",
        "pending",
    ] {
        let record = minimal_record(outcome);
        assert_valid(&record_validator, &serde_json::to_value(&record).unwrap());
    }

    // RuntimeContext round-trip stays opaque (no socket fields).
    let mut linked = minimal_record("completed");
    linked.work_session_id = Some("ws_test".into());
    linked.runtime_context = Some(RuntimeContext {
        provider: "direct".into(),
        workspace_id: None,
        tab_id: None,
        pane_id: None,
    });
    assert_valid(&record_validator, &serde_json::to_value(&linked).unwrap());
}

#[test]
fn legacy_entrypoint_deprecation_and_rejection() {
    let ok = parse_legacy_entrypoint("moon run takogami:build").unwrap();
    assert_eq!(ok.deprecation.code, "legacy_entrypoint_deprecated");
    assert!(parse_legacy_entrypoint("echo hi | cat").is_err());
    assert!(parse_legacy_entrypoint("echo $(whoami)").is_err());
}

#[test]
fn state_home_precedence_and_not_build_session_target() {
    use std::path::Path;
    let build_session = "Build/src/workspaces/wfos/packages/ontarch/registry/sessions";
    let got = resolve_session_state_home(StateHomeInputs {
        cli_state_home: None,
        env_takogami_state_home: None,
        profile_session_state_home: Some("/tmp/operational"),
        env_xdg_state_home: None,
        home_dir: Some(Path::new("/Users/x")),
    });
    assert_eq!(got, PathBuf::from("/tmp/operational"));
    assert_ne!(got, PathBuf::from(build_session));
}

#[test]
fn error_envelope_has_no_secret_or_env_dump() {
    let envelope = CommandEnvelope::error(
        "scan",
        10,
        "not_implemented",
        "command not implemented: scan",
    );
    let text = serde_json::to_string(&envelope).unwrap();
    assert!(!text.contains("AWS_"));
    assert!(!text.contains("password"));
    assert!(!text.contains("\"env\""));
    assert_eq!(envelope.diagnostics[0].code, "not_implemented");
}

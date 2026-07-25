//! Session store / query / recovery tests (no child process spawn).
//!
//! Always uses tempfile roots — never the developer's real state home.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use takogami::contracts::types::{
    ExecutionRecord, OutputSummary, PolicyDecision, RECORD_KIND_COMMAND_EXECUTION, RequestRecord,
    RuntimeCommandRecord, SCHEMA_VERSION,
};
use takogami::sessions::{
    CommandRecordStore, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT, RuntimeContextEnv, SessionStoreError,
    collect_runtime_context, list_sessions, recover_abandoned_pending, show_latest, show_session,
    validate_session_id,
};

fn digest() -> String {
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()
}

fn sample(
    session_id: &str,
    started_at: &str,
    outcome: &str,
    ended_at: Option<&str>,
) -> RuntimeCommandRecord {
    RuntimeCommandRecord {
        schema_version: SCHEMA_VERSION.into(),
        record_kind: RECORD_KIND_COMMAND_EXECUTION.into(),
        session_id: session_id.into(),
        plan_digest: digest(),
        parent_session_id: None,
        work_session_id: None,
        runtime_context: None,
        started_at: started_at.into(),
        ended_at: ended_at.map(str::to_string),
        actor: "agent".into(),
        profile_id: "workspace-dev".into(),
        request: RequestRecord {
            command: "build".into(),
            unit_id: Some("demo".into()),
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
        error: None,
    }
}

#[test]
fn invalid_session_ids_rejected_before_io() {
    let too_long = "x".repeat(129);
    for id in [
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        "a\\b",
        "bad id",
        "has.json",
        too_long.as_str(),
    ] {
        assert!(
            validate_session_id(id).is_err(),
            "expected reject for {id:?}"
        );
        let temp = tempfile::tempdir().unwrap();
        let store = CommandRecordStore::open(temp.path()).unwrap();
        let err = show_session(&store, id).unwrap_err();
        assert!(matches!(err, SessionStoreError::InvalidSessionId));
    }
    assert!(validate_session_id("tkg_1_2_3").is_ok());
    assert!(validate_session_id(&format!("a{}", "_".repeat(127))).is_ok());
}

#[test]
fn atomic_pending_then_final_and_unix_modes() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    let lock = store.acquire_lock("tkg_atomic_1").unwrap();

    let pending = sample("tkg_atomic_1", "2026-07-24T00:00:00Z", "pending", None);
    store.write_pending(&pending, &lock).unwrap();

    let path = store.record_path("tkg_atomic_1").unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'));
    let parsed: RuntimeCommandRecord = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(parsed.execution.outcome, "pending");
    assert!(parsed.ended_at.is_none());

    let mut running = pending;
    running.execution.started = true;
    running.execution.pid = Some(99);
    store.write_final(&running, &lock).unwrap();

    let mut done = running;
    done.execution.outcome = "completed".into();
    done.execution.exit_code = Some(0);
    done.ended_at = Some("2026-07-24T00:00:02Z".into());
    store.write_final(&done, &lock).unwrap();

    let got = store.read_raw("tkg_atomic_1").unwrap();
    assert_eq!(got.execution.outcome, "completed");
    assert_eq!(got.execution.pid, Some(99));
    assert_eq!(got.execution.exit_code, Some(0));

    let root_mode = fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(root_mode, 0o700);
    let locks_mode = fs::metadata(temp.path().join(".locks"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(locks_mode, 0o700);
    let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(file_mode, 0o600);
}

#[test]
fn collision_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    let lock = store.acquire_lock("tkg_collide").unwrap();
    let pending = sample("tkg_collide", "2026-07-24T00:00:00Z", "pending", None);
    store.write_pending(&pending, &lock).unwrap();
    let err = store.write_pending(&pending, &lock).unwrap_err();
    assert!(matches!(err, SessionStoreError::Collision(_)));
}

#[test]
fn symlink_record_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    let target = temp.path().join("target.json");
    fs::write(&target, b"{}\n").unwrap();
    let link = temp.path().join("tkg_symlink.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let err = store.read_raw("tkg_symlink").unwrap_err();
    assert!(matches!(err, SessionStoreError::PathRejected(_)));

    let (list, diag) = list_sessions(&store, Some(50)).unwrap();
    assert!(list.is_empty());
    assert!(diag.skipped.iter().any(|s| s.contains("tkg_symlink")));
}

#[test]
fn abandoned_recovery_when_lock_free() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    {
        let lock = store.acquire_lock("tkg_orphan").unwrap();
        let pending = sample("tkg_orphan", "2026-07-24T00:00:00Z", "pending", None);
        store.write_pending(&pending, &lock).unwrap();
    } // drop releases + removes lock file

    let recovered = recover_abandoned_pending(&store, "tkg_orphan")
        .unwrap()
        .expect("record");
    assert_eq!(recovered.execution.outcome, "abandoned");
    assert!(recovered.ended_at.is_some());
    assert_eq!(
        recovered.error.as_ref().map(|e| e.code.as_str()),
        Some("abandoned_pending")
    );
    let again = store.read_raw("tkg_orphan").unwrap();
    assert_eq!(again.execution.outcome, "abandoned");
}

#[test]
fn active_lock_not_recovered() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    let lock = store.acquire_lock("tkg_active").unwrap();
    let pending = sample("tkg_active", "2026-07-24T00:00:00Z", "pending", None);
    store.write_pending(&pending, &lock).unwrap();

    let still = recover_abandoned_pending(&store, "tkg_active")
        .unwrap()
        .expect("pending");
    assert_eq!(still.execution.outcome, "pending");
    assert!(still.ended_at.is_none());
    drop(lock);
}

#[test]
fn empty_list_and_latest() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    let (list, diag) = list_sessions(&store, None).unwrap();
    assert!(list.is_empty());
    assert!(diag.skipped.is_empty());

    let err = show_latest(&store).unwrap_err();
    assert!(matches!(err, SessionStoreError::NotFound(_)));
}

#[test]
fn list_sorts_newest_first_with_id_tiebreak() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();

    let older = sample(
        "tkg_a",
        "2026-07-24T00:00:00Z",
        "planned",
        Some("2026-07-24T00:00:01Z"),
    );
    let newer = sample(
        "tkg_b",
        "2026-07-24T01:00:00Z",
        "planned",
        Some("2026-07-24T01:00:01Z"),
    );
    let tie_low = sample(
        "tkg_tie_a",
        "2026-07-24T02:00:00Z",
        "planned",
        Some("2026-07-24T02:00:01Z"),
    );
    let tie_high = sample(
        "tkg_tie_z",
        "2026-07-24T02:00:00Z",
        "planned",
        Some("2026-07-24T02:00:01Z"),
    );

    for rec in [&older, &newer, &tie_low, &tie_high] {
        store.write_terminal_unlocked(rec).unwrap();
    }

    let (list, _) = list_sessions(&store, Some(10)).unwrap();
    let ids: Vec<_> = list.iter().map(|s| s.session_id.as_str()).collect();
    assert_eq!(ids, ["tkg_tie_z", "tkg_tie_a", "tkg_b", "tkg_a"]);

    let latest = show_latest(&store).unwrap();
    assert_eq!(latest.session_id, "tkg_tie_z");
    assert_eq!(latest.record_kind, RECORD_KIND_COMMAND_EXECUTION);
}

#[test]
fn list_limit_bounds_and_default() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    for i in 0..60 {
        let id = format!("tkg_{i:03}");
        let rec = sample(
            &id,
            &format!("2026-07-24T00:{i:02}:00Z"),
            "planned",
            Some("2026-07-24T00:00:01Z"),
        );
        store.write_terminal_unlocked(&rec).unwrap();
    }
    let (defaulted, _) = list_sessions(&store, None).unwrap();
    assert_eq!(defaulted.len(), DEFAULT_LIST_LIMIT);

    let (capped, _) = list_sessions(&store, Some(3)).unwrap();
    assert_eq!(capped.len(), 3);

    let err = list_sessions(&store, Some(0)).unwrap_err();
    assert!(matches!(err, SessionStoreError::Contract(_)));
    let err = list_sessions(&store, Some(MAX_LIST_LIMIT + 1)).unwrap_err();
    assert!(matches!(err, SessionStoreError::Contract(_)));
}

#[test]
fn show_absent_and_query_does_not_write_records() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    let before: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert!(before.is_empty());

    let err = show_session(&store, "tkg_missing").unwrap_err();
    assert!(matches!(err, SessionStoreError::NotFound(_)));
    let _ = list_sessions(&store, None).unwrap();
    let _ = show_latest(&store);

    let after: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert!(after.is_empty());
}

#[test]
fn tmp_fragments_ignored_as_records() {
    let temp = tempfile::tempdir().unwrap();
    let store = CommandRecordStore::open(temp.path()).unwrap();
    fs::write(
        temp.path().join(".tmp").join("tkg_frag.1.json"),
        b"{\"not\":\"a record\"}\n",
    )
    .unwrap();
    let (list, _) = list_sessions(&store, None).unwrap();
    assert!(list.is_empty());
}

#[test]
fn runtime_context_herdr_and_tmux() {
    let (ctx, diag) = collect_runtime_context(RuntimeContextEnv {
        herdr_workspace_id: Some("w1"),
        herdr_tab_id: None,
        herdr_pane_id: Some("p2"),
        tmux: Some("/tmp/tmux-1000/default"),
        tmux_pane: Some("%9"),
    });
    assert!(diag.is_none());
    let ctx = ctx.unwrap();
    assert_eq!(ctx.provider, "herdr");
    let wire = serde_json::to_string(&ctx).unwrap();
    assert!(!wire.contains("/tmp/tmux"));
    assert!(!wire.contains("HERDR_SOCKET"));

    let (tmux_ctx, _) = collect_runtime_context(RuntimeContextEnv {
        herdr_workspace_id: None,
        herdr_tab_id: None,
        herdr_pane_id: None,
        tmux: Some("/tmp/tmux-1000/default"),
        tmux_pane: Some("%3"),
    });
    assert_eq!(tmux_ctx.unwrap().pane_id.as_deref(), Some("%3"));
}

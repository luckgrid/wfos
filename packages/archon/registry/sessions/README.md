# Archon registry — session records

Structured, machine-parseable session records, one JSON file per build session
(`<session_id>.json`). Written by the agent at session close per the
[session record schema](../../../../../../../Plan/bin/lg_wfos_session_memory_workflow.md#33-session-record-schema).

These are the durable, queryable twin of the human-readable ledger
[`Build/bin/SESSIONS.md`](../../../../../../bin/SESSIONS.md). A resuming agent reads the **tail** of
the ledger (last 1–3 rows) or queries these records with `jq`; it never replays the full history.

```bash
# examples
jq -r '.story + " " + .status' sessions/*.json          # one-line status per session
jq -s 'sort_by(.ended_at) | last' sessions/*.json        # most recent session record
```

Tracked for provenance (unlike the host-specific, gitignored `registry/tools.json`).

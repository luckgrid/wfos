# Ontarch registry — session records

Structured, machine-parseable session records, one JSON file per build session
(`<session_id>.json`). Written by the agent at session close per the
[session record schema](../../../../../../../../Plan/bin/lg_wfos_session_memory_workflow.md#33-session-record-schema).

**Filename / `session_id` date rule:** use
`YYYY-MM-DD-eNN-sN` where `YYYY-MM-DD` is the **local (PDT) implementation/completion
date** for that story (anchored by nested-repo git history). Do **not** derive the prefix
from a planned sprint date, document mtime, or the next UTC calendar day when
`started_at`/`ended_at` cross midnight. Keep `started_at`/`ended_at` as true UTC instants;
only the filename and `session_id` use the local completion day. The ledger row in
[`Build/bin/wfos/SESSIONS.md`](../../../../../../../bin/wfos/SESSIONS.md) must use the same `session_id`.

These are the durable, queryable twin of the human-readable ledger
[`Build/bin/wfos/SESSIONS.md`](../../../../../../../bin/wfos/SESSIONS.md). A resuming agent reads the **tail** of
the ledger (last 1–3 rows) or queries these records with `jq`; it never replays the full history.

```bash
# examples
jq -r '.story + " " + .status' sessions/*.json          # one-line status per session
jq -s 'sort_by(.ended_at) | last' sessions/*.json        # most recent session record
```

Tracked for provenance (unlike the host-specific, gitignored `registry/tools.json`).

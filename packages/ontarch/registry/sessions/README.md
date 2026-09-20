# Ontarch registry — session records

Structured, machine-parseable session records, one JSON file per build session
(`<session_id>.json`). Written by the agent at session close per the
session record schema in `luckgrid/lg-workstreams`:

- local path: `Build/bin/wfos/program/execution-model.md#33-session-record-schema`
- GitHub: https://github.com/luckgrid/lg-workstreams/blob/main/Build/bin/wfos/program/execution-model.md#33-session-record-schema

**Filename / `session_id` date rule:** use
`YYYY-MM-DD-eNN-sN` where `YYYY-MM-DD` is the **local (PDT) implementation/completion
date** for that story (anchored by nested-repo git history). Do **not** derive the prefix
from a planned sprint date, document mtime, or the next UTC calendar day when
`started_at`/`ended_at` cross midnight. Keep `started_at`/`ended_at` as true UTC instants;
only the filename and `session_id` use the local completion day. The corresponding Workstreams ledger row must use the same `session_id`:

- local path: `Build/bin/wfos/_process/legacy-session-ledger.md`
- GitHub: https://github.com/luckgrid/lg-workstreams/blob/main/Build/bin/wfos/_process/legacy-session-ledger.md

These are the durable, queryable twin of that human-readable Workstreams ledger.
A resuming agent reads the **tail** of the ledger (last 1–3 rows) or queries these
records with `jq`; it never replays the full history.

Paths embedded in existing session JSON are immutable execution evidence and
retain the Workstreams locators that were current when each record was written.

When a historical record refers to Workstreams material that has since moved, add a
`workstreams_refs` array without rewriting the historical field. Each entry carries both
the current local path within `luckgrid/lg-workstreams` and its GitHub URL:

```json
{
  "role": "story",
  "historical_locator": "Build/bin/wfos/E01_dotfiles_shell_substrate.md#s1",
  "local_path": "Build/bin/wfos/program/wfos-e01/s1-adopt-chezmoi-as-the-dotfile-manager.md",
  "github_url": "https://github.com/luckgrid/lg-workstreams/blob/main/Build/bin/wfos/program/wfos-e01/s1-adopt-chezmoi-as-the-dotfile-manager.md"
}
```

Use this current-reference surface only for navigational/provenance references such as
`loaded_context`, `plan`, `addendum`, or an equivalent source-spec field. Do not
mechanically duplicate every path that appears in outcome prose, command output,
`changed_paths`, or other immutable execution evidence.

```bash
# examples
jq -r '.story + " " + .status' sessions/*.json          # one-line status per session
jq -s 'sort_by(.ended_at) | last' sessions/*.json        # most recent session record
```

Tracked for provenance (unlike the host-specific, gitignored `registry/tools.json`).

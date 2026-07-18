# Bin/archive lifecycle

`bin/` holds disposable workflow outputs and scratch artifacts. `lib/` and `src/` hold
reviewable reference and maintained source. The rule is simple: **bin is disposable but
traceable** — a small manifest answers "what is this, can I delete it" without inspecting
output files, and a report-only inventory replaces ad-hoc `du`/`ls`/`stat` exploration.

## Inventory (report-only)

```bash
moon run archon:bin-report
# or: packages/archon/bin/archon bin-report
```

The report walks every namespace `bin/<workflow>/` under the Workstreams root
(`Plan/bin`, `Build/bin`, `Control/bin`, …). For each workflow directory it records:

| Field | Meaning |
|-------|---------|
| `path` | Relative path from the Workstreams root |
| `size_bytes` | Total size (`du -sk`, converted to bytes) |
| `file_count` | Number of files (`fd`, with `find` fallback) |
| `oldest_file_age_days` | Age of the oldest file (days), or null if empty |
| `newest_file_age_days` | Age of the newest file (days), or null if empty |
| `manifest_present` | Whether any `manifest.json` exists under the tree |
| `manifest_count` | How many `manifest.json` files were found |

Outputs land in the Archon registry (host-specific, gitignored):

- `packages/archon/registry/bin-inventory.json` — machine-readable
- `packages/archon/registry/BIN-INVENTORY.md` — RTK-compressible table

The inventory is **read-only**: it never writes under `bin/`, never deletes, and never moves.

## Manifests

Every non-trivial Archon-generated run carries a `manifest.json` beside its outputs. Day-one
scope is Archon's own generated artifacts (`registry/*.json`, scan, graph). Other `bin/`
writers are advised to emit the same shape; the schema validates any manifest that exists
but does not require one outside Archon.

Required fields: `id`, `workflow`, `source`, `created_at`, `tool`, `outputs`, `retention`.

Retention values:

| Value | Meaning |
|-------|---------|
| `review-before-delete` | Safe default — human reviews before purge |
| `auto-archive-after:<N>d` | Eligible for archive after N days (e.g. `auto-archive-after:30d`) |
| `permanent` | Never auto-delete |
| `session-exports` | Session export retention — review-before-delete posture |

See `packages/archon/schemas/manifest.schema.json` and the fixture at
`packages/archon/registry/fixtures/example-manifest.json`.

## Cleanup modes

Cleanup never removes user-owned work silently. Modes (implemented by
`moon run archon:bin-cleanup` / `archon bin-cleanup`):

| Mode | Behavior |
|------|----------|
| `report-only` (default) | Print inventory + stale candidates; no action |
| `dry-run` | Print an exact plan of what would move/delete; exit 0; no action |
| `archive` | Move stale items to archive paths and update manifest fields (human-only; deferred at draft gateway) |
| `delete-approved` | Delete only items whose `approved_to` matches `--scope` and whose retention is not `permanent` (human-only; deferred at draft gateway) |

Blocked in all modes: `rm -rf` globs, `git clean`, deleting untracked files without a
manifest, deleting `lib/` or `src/` material, deleting anything with `retention: "permanent"`.

At the current draft gateway, `archive` and `delete-approved` validate arguments and then
refuse (no filesystem mutation). Agents under `DUST_AGENT=1` are refused those modes
outright. Real archive/delete execution is deferred to later automation (Kraken / H12).

## Archive reasons and promotion

Archive reason labels (recorded on the manifest as `archive_reason`):

| Reason | Meaning |
|--------|---------|
| `superseded` | Replaced by a newer version |
| `imported` | Absorbed into `src/` or `lib/` |
| `retired` | No longer active; history matters |
| `reference` | Kept for cross-reference; not maintained |
| `duplicate` | Redundant copy |
| `stale` | Age exceeds useful lifetime |

Promotion routes (documented, not automated at draft state). Optional `promoted_to` on the
manifest records the destination when applicable:

| Route | Meaning |
|-------|---------|
| `bin → src` | Requires review, stable name, frontmatter/descriptor where applicable |
| `bin → lib` | Durable reference material, not canonical source |
| `src → src/archives` | Retired canonical source; history matters |
| `lib → src` | Reference becomes maintained source |

## Agent rails

Report-only inventory is agent-safe. Cleanup mutation (`archive`, `delete-approved`) is
human-only. The `agent-bin` Archon policy records allow/gate/block tiers for bin/archive
commands; see [agent-rails.md](agent-rails.md). Runtime command interception is deferred to
Kraken — the same boundary as git and secret rails.

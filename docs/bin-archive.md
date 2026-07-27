# Bin/archive lifecycle

`bin/` holds disposable workflow outputs and scratch artifacts. `lib/` and `src/` hold
reviewable reference and maintained source. The rule is simple: **bin is disposable but
traceable** — a small manifest answers "what is this, can I delete it" without inspecting
output files, and a report-only inventory replaces ad-hoc `du`/`ls`/`stat` exploration.

## Inventory (report-only)

```bash
moon run ontarch:bin-report
# or: packages/ontarch/bin/ontarch bin-report
```

The report walks every namespace `bin/<workflow>/` under the Workstreams root
(`Plan/bin`, `Build/bin`, `Control/bin`, …).

These namespace bin roots are inventory *walk* roots, not accepted Takogami explicit `--scope` values. For each workflow directory it records:

| Field | Meaning |
|-------|---------|
| `path` | Relative path from the Workstreams root |
| `size_bytes` | Total size (`du -sk`, converted to bytes) |
| `file_count` | Number of files (`fd`, with `find` fallback) |
| `oldest_file_age_days` | Age of the oldest file (days), or null if empty |
| `newest_file_age_days` | Age of the newest file (days), or null if empty |
| `manifest_present` | Whether any `manifest.json` exists under the tree |
| `manifest_count` | How many `manifest.json` files were found |

Outputs land in the metadata-plane (Ontarch) registry (host-specific, gitignored):

- `packages/ontarch/registry/bin-inventory.json` — machine-readable
- `packages/ontarch/registry/BIN-INVENTORY.md` — RTK-compressible table

The inventory is **read-only**: it never writes under `bin/`, never deletes, and never moves.

## Manifests

Every non-trivial metadata-plane-generated run carries a `manifest.json` beside its outputs. Day-one
scope is the metadata-plane's own generated artifacts (`registry/*.json`, scan, graph). Other `bin/`
writers are advised to emit the same shape; the schema validates any manifest that exists
but does not require one outside the metadata-plane (Ontarch).

Required fields: `id`, `workflow`, `source`, `created_at`, `tool`, `outputs`, `retention`.

Retention values:

| Value | Meaning |
|-------|---------|
| `review-before-delete` | Safe default — human reviews before purge |
| `auto-archive-after:<N>d` | Eligible for archive after N days (e.g. `auto-archive-after:30d`) |
| `permanent` | Never auto-delete |
| `session-exports` | Session export retention — review-before-delete posture |

See `packages/ontarch/schemas/manifest.schema.json` and the fixture at
`packages/ontarch/registry/fixtures/example-manifest.json`.


## Takogami `--scope` (D9 Option B)

When routing cleanup through the runtime controller (`takogami bin cleanup`), an explicit
`--scope` must be a workflow/subtree path:

```text
<namespace>/bin/<segment>[/<segment>...]
```

Examples: `Plan/bin/research`, `Build/bin/wfos`, `Build/bin/wfos/reviews`.
Namespace roots such as `Plan/bin` or `Build/bin` are **not** valid explicit scopes.
Omitting `--scope` keeps workspace-wide non-mutating report/planning behavior.
The Phase 1 Ontarch schema grammar is unchanged.

## Cleanup modes

Cleanup never removes user-owned work silently. Modes (implemented by
`moon run ontarch:bin-cleanup` / `ontarch bin-cleanup`):

| Mode | Behavior |
|------|----------|
| `report-only` (default) | Print inventory + stale candidates; no action |
| `dry-run` | Print an exact plan of what would move/delete; exit 0; no action |
| `archive` | Move stale items to archive paths and update manifest fields (human-only; deferred at draft gateway) |
| `delete-approved` | Delete only items whose `approved_to` matches `--scope` and whose retention is not `permanent` (human-only; deferred at draft gateway) |

Blocked in all modes: `rm -rf` globs, `git clean`, deleting untracked files without a
manifest, deleting `lib/` or `src/` material, deleting anything with `retention: "permanent"`,
and any workflow with more than one `manifest.json`.

Operation order (locked): parse options → validate mode/scope → locate inventory →
validate existing inventory (fail closed; no overwrite) or refresh when missing →
build plan → validate plan → emit. Invalid/missing scope or options never write registry
outputs. Missing inventory refreshes via validated `bin-report` and sets
`inventory_refreshed=true`; a valid existing inventory sets `inventory_refreshed=false`.

At the current draft gateway, `archive` and `delete-approved` validate arguments and then
refuse (no filesystem mutation). Agents under `PANOPLY_AGENT=1` are refused those modes
outright. Real archive/delete execution is deferred to later automation (runtime-controller (Takogami) / H12).

## Cleanup classification (Phase 1)

Inventory `manifest_count` is authoritative. Exactly one manifest under the workflow is
accepted; more than one is `blocked` / `multiple-manifests`. Staleness uses
`newest_file_age_days` (conservative: recently updated trees stay current).

| Condition | report-only | dry-run (archive) |
|-----------|-------------|-------------------|
| `manifest_count == 0` | `advisory` / `no-manifest` | `blocked` / `no-manifest` |
| `manifest_count > 1` | `blocked` / `multiple-manifests` | `blocked` / `multiple-manifests` |
| `retention == permanent` | `blocked` / `retention-permanent` | `blocked` / `retention-permanent` |
| `auto-archive-after:Nd` and newest age ≥ N | `advisory` / `stale` | `would_archive` / `stale` |
| `auto-archive-after:Nd` and newest age < N (or null) | `advisory` / `current` | `advisory` / `current` |
| `review-before-delete` / `session-exports` | `advisory` / `retention-review-required` | `advisory` / `retention-review-required` |

Delete overlay (dry-run with `--scope`, or deferred `delete-approved` plan):

| Condition | disposition / reason |
|-----------|----------------------|
| no `--scope` | `blocked` / `scope-required` |
| path outside scope | `blocked` / `outside-scope` |
| no / multiple / invalid manifest | `blocked` / matching reason |
| `approved_to` null | `blocked` / `approved-to-null` |
| `approved_to` ≠ scope | `blocked` / `approved-to-mismatch` |
| exactly one manifest, not permanent, `approved_to` equals scope | `would_delete` / `approved` |

Closed cleanup `reason` vocabulary: `approved`, `approved-to-mismatch`, `approved-to-null`,
`current`, `invalid-manifest`, `lib-or-src`, `multiple-manifests`, `no-manifest`,
`outside-scope`, `retention-permanent`, `retention-review-required`, `scope-required`,
`stale`.

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
human-only. The `agent-bin` metadata-plane (Ontarch) policy records allow/gate/block tiers for bin/archive
commands; see [agent-rails.md](agent-rails.md). Runtime command interception is deferred to
the runtime-controller (Takogami) — the same boundary as git and secret rails.

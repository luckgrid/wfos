# `runtime-controller` — Takogami

The runtime-controller (Takogami) is the WfOS runtime CLI (`takogami`): discovery, routing,
policy, command execution records, and explain output. It coordinates the
[native-toolchain (Panoply)](../panoply/README.md) and
[metadata-plane (Ontarch)](../ontarch/README.md); it does not replace them.

It does not own persistent terminal PTYs (tmux / optional Herdr) or desktop window restore.

**Status: E09 runtime-controller MVP implemented.** Lifecycle `dev` / `build` / `check`
resolve a sealed plan, evaluate dual-layer profile/policy rules (request + child), and emit
Allow / Gate / Deny with safe provenance. Resolution is plan-only unless `--execute` is
supplied. Plan-only never starts the resolved child, but a policy-decision-bearing attempt
persists a terminal `RuntimeCommandRecord` with `outcome=planned`, `started=false`, and
`pid=null`. Allowed direct `--execute` writes a durable pending `RuntimeCommandRecord`
(schema `0.1.0`), then runs the sealed child through the single hardened Tokio executor with
literal argv and `env_clear` + sealed non-sensitive keys. `session list|show|latest` queries
those operational command-execution records (including planned). `takogami graph` projects the
Ontarch registry graph (zero-spawn, no operational record, no implicit sync). Supported bin
projections (`bin report`, cleanup `report-only`) share the same executor and record pipeline;
cleanup `dry-run` is Gate/no-spawn; cleanup `archive` / `delete-approved` are Deny +
`deferred_unavailable` with no spawn. Optional RTK postprocesses eligible human streams only.
Interactive providers and work-session restore remain post-MVP.

## Build

From the workspace root:

```bash
moon run takogami:build
moon run takogami:test
moon run takogami:lint
moon run takogami:format-check
```

## Command surface

```txt
takogami --version | --help
takogami doctor [--json]
takogami scan [--refresh] [--json]
takogami list units|tools [--filter FIELD=VALUE] [--json]
takogami info <unit> [--json]
takogami tools [--json]
takogami interfaces [--validate] [--json]
takogami dev|build|check <unit> [--explain] [--execute] [--json]
  → resolve + dual-layer policy; plan-only unless --execute
  → only evaluator-minted dual-Allow authorization reaches execution
  → policy deny exit 5; policy gate exit 6 (fail closed; no approval bypass)
  → child exit codes pass through; state I/O exit 7; execution I/O exit 8
takogami session list [--limit N] [--json]
takogami session show <session-id> [--json]
takogami session latest [--json]
  → operational command_execution records only (including planned; not build or work sessions)
takogami graph [--format text|dot|json] [--json]
  → typed registry graph projection; hit/miss/stale; no child; no operational record
takogami bin report [--json]
  → dual-Allow → Ontarch child executes once → terminal record
takogami bin cleanup --mode report-only|dry-run|archive|delete-approved [--scope SCOPE] [--json]
  → report-only: dual-Allow / execute / record
  → dry-run: Gate / no spawn
  → archive|delete-approved: Deny + deferred_unavailable / no spawn
```

Global flags: `--json`, `--profile`, `--state-home`, `--no-color`, `--verbose`.

Registry override for tests/fixtures: `TAKOGAMI_ONTARCH_REGISTRY`, `TAKOGAMI_WORKSPACE_ROOT`.

### Graph projection

- Reads `registry/graph.json` only (sibling `graph.dot` is never trusted).
- Layered freshness: graph upstream fingerprints resolve under `TAKOGAMI_ONTARCH_REGISTRY`
  (`registry_root`); authored unit fingerprints resolve under `TAKOGAMI_WORKSPACE_ROOT`.
- Hit / miss / stale are typed exits. Miss or stale never syncs implicitly — run
  `moon run ontarch:sync` (or `ontarch sync`) explicitly, then re-query.
- Formats: `--format text` (default), `dot`, or `json`. Global `--json` wraps one
  `CommandEnvelope` with structured `data.graph`.
- Zero child process and zero operational command record on every graph path.
- Graph machine JSON is never RTK transformed.
- Internal limits (fail closed): 8 MiB graph file, 20k nodes, 100k edges, 512-byte IDs.

### Bin projection

| Operation | Request/child decision | Ontarch child | Record outcome |
|-----------|------------------------|---------------|----------------|
| `bin report` | Allow / Allow | executes once | completed / controller error / failure as truthful |
| cleanup `report-only` | Allow / Allow | executes once | completed / controller error / failure as truthful |
| cleanup `dry-run` | Gate | no spawn | gated |
| cleanup `archive` | Deny + `deferred_unavailable` | no spawn | denied |
| cleanup `delete-approved` | Deny + `deferred_unavailable` | no spawn | denied |

- Takogami seals canonical Ontarch identity and a controller-owned helper PATH; caller `PATH`
  has no authority over Ontarch/helper selection.
- Projection children receive controller-owned `PANOPLY_AGENT=1`.
- Child machine JSON is bounded and contract-validated; it is never RTK transformed.
- Full inventory/plan payloads are not persisted in command records.
- Explicit `--scope` requires `namespace/bin/<segment>[/<segment>...]` (e.g.
  `Build/bin/wfos`). Namespace roots (`Plan/bin`, `Build/bin`), absolute paths, and traversal
  are invalid (`bin_scope_invalid`, usage exit 2). Omitting `--scope` keeps workspace-wide
  non-mutating report/planning behavior.
- Archive/delete mutation is not available in E09.

### Lifecycle resolution

- Profile precedence: CLI `--profile` → `TAKOGAMI_PROFILE` → `workspace-dev` → fail closed.
- No shell: structured argv boundaries preserved; legacy strings use the constrained parser.
- Plan-only without `--execute`: resolution never starts the resolved child (no Panoply,
  Ontarch, Herdr, or tmux spawn), but a policy-decision-bearing attempt persists a terminal
  `RuntimeCommandRecord` with `outcome=planned`, `started=false`, and `pid=null`. Graph
  remains the distinct zero-spawn and no-record path.
- With `--execute`: only evaluator-minted dual-Allow authorization reaches the hardened
  executor; pending state is persisted before spawn; child PID, streams, signals, exit, and
  terminal outcome are recorded truthfully.
- Authored descriptor TOML is authoritative on stale/miss; `units.json` is a cache.
- Authored routing structures are closed contracts; malformed or ambiguous candidates fail closed.
- Selected manifests must match exact declared canonical identities; basename equality is not
  authorization.
- Native/Moon use ordered, deduplicated `PATH`; unordered Panoply candidates fail as
  `executable_ambiguous`.
- Failure explanations stop at the exact failed step and never invent a plan digest.
- Policy: fixed actor=`agent`; Deny > Gate > Allow across request and child layers; default deny;
  profiles may narrow but never weaken a cross-cutting block; Gate fails closed (no CLI/env/file
  approval bypass). Malformed policy is exit 3 (contract).
- Interactive execution classes remain unavailable (exit 10).

`takogami build <unit>` is the unit lifecycle verb. A separate `workstream` namespace is
post-MVP. `takogami session *` reads **command execution records**, not composed work sessions.

## Controller exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Internal / unavailable source |
| 2 | Usage (invalid flags, not-found, ambiguous, invalid-filter, `bin_scope_invalid`) |
| 3 | Contract / invalid-registry / policy-contract / graph-contract / payload invalid |
| 4 | Resolution / generated-state freshness failure (`graph_missing`, `graph_stale`, …) |
| 5 | Policy deny |
| 6 | Policy gate (fail closed) |
| 7 | State I/O |
| 8 | Execution I/O |
| 10 | Not implemented / execution_unavailable / execution_class_unavailable |

Native child exit codes pass through on successful spawn paths and are distinct from these
controller categories.

### Stable S7 diagnostics (operator remediation)

Documented codes that affect remediation. Exact prose strings are not API contracts.

```txt
graph_missing
graph_stale
graph_contract_invalid
graph_endpoint_invalid
graph_limit_exceeded
bin_scope_invalid          (usage exit 2; code in message text)
bin_payload_invalid
bin_inventory_invalid
bin_cleanup_plan_invalid
deferred_unavailable
projection_contract_changed
projection_tool_unavailable
state_io
execution_io
```

## Freshness (S3)

Reads of `units.json` / `scan.json` compare embedded `registry_generation.source_fingerprints`
to current source bytes → `hit` / `miss` / `stale`. Missing generation metadata is `stale`.
`--refresh` on `scan` invokes Ontarch scan explicitly; read-only queries never refresh as a
side effect. Envelope `metrics.registry_cache` carries the label in JSON mode.

## Doctor (S3)

Required: `cargo` / `rustc` / `moon` on PATH, registry contract readability, state-home
writability (probe only — no command record). Optional: `rtk`, `tmux`, `herdr` — missing Herdr
never fails base doctor. Takogami may report readiness but does not own or start tmux/Herdr
servers in E09.

## Optional RTK

RTK applies only to eligible human lifecycle output. Graph/bin machine JSON remains
uncompressed. Truthful fallback is recorded when RTK is absent or unsupported.

Design: [`../../docs/runtime-controller.md`](../../docs/runtime-controller.md) ·
engine: [`../../docs/runtime-architecture.md`](../../docs/runtime-architecture.md).

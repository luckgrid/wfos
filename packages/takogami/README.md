# `runtime-controller` — Takogami

The runtime-controller (Takogami) is the WfOS runtime CLI (`takogami`): discovery, routing,
policy, command execution records, and explain output. It coordinates the
[native-toolchain (Panoply)](../panoply/README.md) and
[metadata-plane (Ontarch)](../ontarch/README.md); it does not replace them.

It does not own persistent terminal PTYs (tmux / optional Herdr) or desktop window restore.

**Status: E09.S2.1 — command-record contract.** Public type/schema is
`RuntimeCommandRecord` / `runtime-command-record.schema.json` with
`record_kind: command_execution`. S3 discovery/queries/doctor remain. Lifecycle
resolve/execute, policy, graph/bin, and `session *` remain `not_implemented`
until later stories. Next: **E09.S4** (deterministic resolution).

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
takogami dev|build|check|graph|bin|session …
  → not_implemented (exit 10) until later E09 stories
```

Global flags: `--json`, `--profile`, `--state-home`, `--no-color`, `--verbose`.

Registry override for tests/fixtures: `TAKOGAMI_ONTARCH_REGISTRY`, `TAKOGAMI_WORKSPACE_ROOT`.

`takogami build <unit>` is the unit lifecycle verb. A separate `workstream` namespace is
post-MVP. `takogami session *` (S6) reads **command execution records**, not composed work
sessions.

## Controller exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Internal / unavailable source |
| 2 | Usage (invalid flags, not-found, ambiguous, invalid-filter) |
| 3 | Contract / invalid-registry |
| 4 | Resolution failure (reserved) |
| 5 | Policy deny (reserved) |
| 6 | Policy gate (reserved) |
| 10 | Not implemented |

## Freshness (S3)

Reads of `units.json` / `scan.json` compare embedded `registry_generation.source_fingerprints`
to current source bytes → `hit` / `miss` / `stale`. Missing generation metadata is `stale`.
`--refresh` on `scan` invokes Ontarch scan explicitly; read-only queries never refresh as a
side effect. Envelope `metrics.registry_cache` carries the label in JSON mode.

## Doctor (S3)

Required: `cargo` / `rustc` / `moon` on PATH, registry contract readability, state-home
writability (probe only — no command record). Optional: `rtk`, `tmux`, `herdr` — missing Herdr
never fails base doctor.

Design: [`../../docs/runtime-controller.md`](../../docs/runtime-controller.md) ·
engine: [`../../docs/runtime-architecture.md`](../../docs/runtime-architecture.md).

# `runtime-controller` — Takogami

The runtime-controller (Takogami) is the WfOS runtime CLI (`takogami`): discovery, routing,
policy, command execution records, and explain output. It coordinates the
[native-toolchain (Panoply)](../panoply/README.md) and
[metadata-plane (Ontarch)](../ontarch/README.md); it does not replace them.

It does not own persistent terminal PTYs (tmux / optional Herdr) or desktop window restore.

**Status: E09.S2 — metadata and runtime contracts.** The binary builds and exposes the full
MVP command tree; only `doctor` is implemented. Other commands return a structured
`not_implemented` error (human + `--json`). Typed contracts for envelopes, operational records
(today `RuntimeSession`; rename to `RuntimeCommandRecord` in E09.S2.1), resolved commands,
policy decisions, legacy entrypoints, fingerprints, and state-home precedence are in place for
later stories. Next implementation story: **E09.S3** discovery/queries/complete doctor.

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
takogami scan | list | info | tools | interfaces | dev | build | check | graph | bin | session …
  → not_implemented (exit 10) until later E09 stories
```

Global flags: `--json`, `--profile`, `--state-home`, `--no-color`, `--verbose`.

`takogami build <unit>` is the unit lifecycle verb. A separate `workstream` namespace is
post-MVP. `takogami session *` (S6) reads **command execution records**, not composed work
sessions.

## Controller exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Internal error |
| 2 | Usage (invalid flags / unknown command) |
| 3 | Contract / schema-version failure |
| 4 | Resolution failure (reserved) |
| 5 | Policy deny (reserved) |
| 6 | Policy gate (reserved) |
| 10 | Not implemented |

Native child exit codes pass through unchanged in later stories (S6).

## Contracts (S2)

Machine contracts live under `src/contracts/` and Ontarch schemas:

- `packages/ontarch/schemas/command-output.schema.json` — `CommandEnvelope`
- `packages/ontarch/schemas/runtime-session.schema.json` — operational record (S2 name;
  **E09.S2.1** renames to `runtime-command-record.schema.json` / `RuntimeCommandRecord`)
- Structured lifecycle entrypoints in `unit.schema.json` (legacy strings still accepted)
- Operational state home: `--state-home` → `TAKOGAMI_STATE_HOME` → profile
  `[runtime] session_state_home` → XDG → `~/.local/state/takogami/sessions`
- `logs.session_log_target` remains tracked build-session provenance only

## Doctor (S1 skeleton)

Checks only that `cargo`, `rustc`, and `moon` are on `PATH`. Does **not** claim registry,
command-record store, or RTK readiness (those arrive in E09.S3). Missing optional tools such as
Herdr must not fail the base doctor once S3 lands.

Design: [`../../docs/runtime-controller.md`](../../docs/runtime-controller.md) ·
engine: [`../../docs/runtime-architecture.md`](../../docs/runtime-architecture.md).

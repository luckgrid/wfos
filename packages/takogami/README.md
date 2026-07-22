# `runtime-controller` — Takogami

The runtime-controller (Takogami) is the WfOS runtime CLI (`takogami`): discovery, routing,
policy, command execution records, and explain output. It coordinates the
[native-toolchain (Panoply)](../panoply/README.md) and
[metadata-plane (Ontarch)](../ontarch/README.md); it does not replace them.

It does not own persistent terminal PTYs (tmux / optional Herdr) or desktop window restore.

**Status: deterministic resolution hardening complete (plan-only).** Lifecycle `dev` / `build` /
`check` resolve a sealed pre-policy plan and optional `--explain` output. Failed resolution emits
a safely redacted partial trace; successful plans bind canonical executable, cwd, manifest, and
selection provenance into the plan digest. The commands do **not** spawn processes, evaluate
policy, or persist `RuntimeCommandRecord`. `--execute` returns `execution_unavailable` (exit 10).
Non-`direct` execution classes return `execution_class_unavailable` (exit 10). Next: policy
enforcement.

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
  → plan-only resolution; --execute → execution_unavailable
takogami graph|bin|session …
  → not_implemented (exit 10) until later runtime-controller phases
```

Global flags: `--json`, `--profile`, `--state-home`, `--no-color`, `--verbose`.

Registry override for tests/fixtures: `TAKOGAMI_ONTARCH_REGISTRY`, `TAKOGAMI_WORKSPACE_ROOT`.

### Lifecycle resolution

- Profile precedence: CLI `--profile` → `TAKOGAMI_PROFILE` → `workspace-dev` → fail closed.
- No shell: structured argv boundaries preserved; legacy strings use the constrained parser.
- No spawn: resolution never runs the resolved executable, Panoply, Ontarch, Herdr, or tmux.
- No operational state: `--state-home` is ignored for writes; no command-record files.
- Authored descriptor TOML is authoritative on stale/miss; `units.json` is a cache.
- Authored routing structures are closed contracts; malformed or ambiguous candidates fail closed.
- Selected manifests must match exact declared canonical identities; basename equality is not
  authorization.
- Native/Moon use ordered, deduplicated `PATH`; unordered Panoply candidates fail as
  `executable_ambiguous`.
- Failure explanations stop at the exact failed step and never invent a plan digest.

`takogami build <unit>` is the unit lifecycle verb. A separate `workstream` namespace is
post-MVP. The future `takogami session *` surface reads **command execution records**, not composed work
sessions.

## Controller exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Internal / unavailable source |
| 2 | Usage (invalid flags, not-found, ambiguous, invalid-filter) |
| 3 | Contract / invalid-registry |
| 4 | Resolution failure |
| 5 | Policy deny (reserved) |
| 6 | Policy gate (reserved) |
| 10 | Not implemented / execution_unavailable / execution_class_unavailable |

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

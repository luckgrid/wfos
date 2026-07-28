# E09 MVP end-to-end fixture (S7 Phase 4)

Tracked source templates only. Tests copy into a temp root and must not mutate
these paths.

## Layout

- `workspace/` — Workstreams-like tree with bin candidates (permanent demo,
  stale-demo, missing-manifest, scope-mismatch) and descriptor-backed /
  descriptor-less unit path placeholders
- `ontarch/registry/` — coherent graph + registry inputs (fingerprints match)
- `tools/` — documented hermetic PATH root (shims written at runtime)
- `state/` — empty placeholder; harness uses a temp state-home instead
- `variants/` — stale and malformed graph/units overlays for fail-closed tests
- `expected/` — graph byte goldens plus structural bin/cleanup envelope shapes

## Graph expected outputs

Files under `expected/graph-*` are complete canonical projections generated from
`ontarch/` for byte comparison in `graph_cli`:

- `graph-text.txt` — human text (`takogami graph`)
- `graph-dot.txt` — DOT (`takogami graph --format dot`)
- `graph-envelope.json` — global JSON envelope (`takogami --json graph --format text`)

`expected/bin-report-envelope.json` and `expected/cleanup-report-envelope.json`
are structural shapes (workspace root redacted as `<workspace>`); integrated
tests assert schema fields and `compressor: "none"` rather than absolute paths.

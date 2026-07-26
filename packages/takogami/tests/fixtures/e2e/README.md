# E09 MVP end-to-end fixture (S7)

Tracked source templates only. Tests copy into a temp root and must not mutate
these paths.

## Graph expected outputs (Phase 2)

Files under `expected/graph-*` are complete canonical projections generated from
`ontarch/` (workspace root + `registry/`) for byte comparison in `graph_cli`:

- `graph-text.txt` — human text (`takogami graph`)
- `graph-dot.txt` — DOT (`takogami graph --format dot`)
- `graph-envelope.json` — global JSON envelope (`takogami --json graph --format text`)

Bin/cleanup expected envelopes remain Phase 3 placeholders.

# Agents navigation pattern

Reusable **seed** for a workspace agent navigation layer. It is data (YAML/TOML/Markdown),
not code. `ontarch agents-init` materializes it into a working `$AGENTS_HOME` (default
`<workspace-root>/.agents/`).

## Seed vs working copy

| Concern | Home |
|---------|------|
| Machine routing, policies, schemas, registry | Ontarch package (`descriptors/`, `policies/`, `schemas/`, `registry/`) |
| **Pattern seed** (this directory) | `packages/ontarch/patterns/agents/` |
| **Working navigation layer** | `$AGENTS_HOME` (usually `<repo>/.agents/`) — operator-edited profiles/skills |

Ontarch descriptors and policies remain the **machine-routing authority**. The working
`.agents/` directory is where an operator (or agent) orients: which profiles exist, which
tools are present/missing, which skills are mapped, and how the pieces relate.

## Layout (seed)

```txt
patterns/agents/
├── PATTERN.toml           # id + version (paired with working .pattern-lock)
├── profiles/              # README contract, AGENTS.template.md, examples/
├── skills/                # README contract, examples/, templates/
├── tools/README.md        # local-toolkit.yml contract (file itself is generated)
└── graphs/README.md       # pointer to Ontarch graph schema + registry
```

## Materialization

```bash
moon run ontarch:agents-init
# or: bin/ontarch agents-init [--target DIR] [--force]
```

Init copies contracts and example records into `$AGENTS_HOME`, writes `.pattern-lock`, and
refuses to overwrite existing files unless `--force`. Seed-owned templates
(`profiles/AGENTS.template.md`, `skills/templates/*`) stay in this pattern directory —
`ontarch skills resolve` / `validate` fall back here. `local-toolkit.yml` is never part of the
seed — `ontarch sync` generates it.

## Related

- Schemas: `../../schemas/profile.schema.json`, `../../schemas/skill.schema.json`
- Policies: `../../policies/`
- Docs: `../../../docs/agent-configs.md`, `../../../docs/agent-skills.md`

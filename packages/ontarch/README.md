# `metadata-plane` — Ontarch 📐

Ontarch stores WfOS's machine-readable meaning: descriptors, registries, schemas, policies,
profiles, graphs, patterns, and package contracts. It is data and build-time tooling rather than
an end-user runtime. Other packages read its contracts and generated projections.

This README is the entrypoint for every worker touching the package. Deep reference:
[`../../docs/metadata-plane.md`](../../docs/metadata-plane.md).

## Authority and safety

- Authored descriptors, schemas, policies, profiles, and pattern contracts are source material.
- Generated registry files and graphs are projections; regenerate them instead of hand-editing
  them.
- Native manifests remain authoritative for build, dependency, and package behavior.
- Reading contracts is safe. Mutations must stay within the selected profile and policy.
- `moon run ontarch:validate` is the package gate.

## Tasks

| Task | Purpose |
|---|---|
| `moon run ontarch:validate` | Validate descriptors, policies, profiles, skills, graphs, and bin contracts |
| `moon run ontarch:sync` | Generate unit, skill, profile, policy, tool, and graph projections |
| `moon run ontarch:scan` | Emit the read-only polyrepo scan report |
| `moon run ontarch:bin-report` | Emit bin inventory JSON and Markdown projections |
| `moon run ontarch:bin-cleanup` | Produce report, dry-run, archive, or approved-delete cleanup plans under policy |
| `moon run ontarch:agents-init` | Seed a working `$AGENTS_HOME` from `patterns/agents/` |

The tasks use bash, `awk`, and `jq`. Registry writers modify only generated output under
`registry/`; `agents-init` modifies only the selected navigation layer.

## Package map

| Path | Role |
|---|---|
| `descriptors/` | Central descriptors and overrides |
| `schemas/` | Unit, policy, profile, skill, command, runtime-record, and bin contracts |
| `policies/` | Reusable command, secret, publish, runtime, and bin rails |
| `graphs/` | Generated graph contract |
| `patterns/agents/` | Reusable profile, skill, tool, and graph navigation seed |
| `bin/`, `lib/` | Ontarch task entrypoints and shared parsing/generation helpers |
| `registry/` | Generated indexes and graphs plus tracked build-session provenance |
| `registry/QUERIES.md` | jq query cookbook over generated data |

## Core contracts

```text
Descriptors  describe how units connect.
Registries   index what exists.
Schemas      define machine contracts.
Policies     define reusable allow, gate, and block intent.
Profiles     scope policies to an automated session.
Graphs       expose capability, dependency, policy, and profile relationships.
Patterns     seed reusable navigation and configuration contracts.
Models       define domain meaning (planned).
Packages     define package-translator interfaces (planned).
```

## Generated data rule

The registry is host-specific and mostly gitignored:

```text
registry/units.json
registry/skills.json
registry/profiles.json
registry/policies.json
registry/tools.json
registry/graph.json
registry/graph.dot
registry/bin-inventory.json
registry/BIN-INVENTORY.md
```

Regenerate these with `ontarch sync`, `panoply doctor`, or the appropriate report task. Do not
edit them by hand. `registry/sessions/*.json` is different: those tracked files are authored
build-session provenance records.

## Automated-worker profiles

Working profiles are authored under `$AGENTS_HOME/profiles/`, commonly
`Workstreams/.agents/profiles/` in an embedded layout. Ontarch validates them against
`schemas/profile.schema.json`, resolves their selected policies and skills, emits compact registry
data, and draws relationship edges.

The reusable seed is [`patterns/agents/`](patterns/agents/README.md). It does not create a
repository instruction file. Repository and package orientation belongs in README files; the
pattern supplies automated-worker profile, skill, tool, and graph contracts.

## Editing rules

- Follow the schema-compatible TOML subset used by Ontarch's parser.
- Keep descriptor and policy IDs stable unless an explicit migration changes them.
- Add generated artifact schemas before consumers depend on those artifacts.
- Keep policy metadata honest about current enforcement boundaries; do not claim runtime
  enforcement that is still deferred.
- Keep registry paths and outputs out of authored package contracts when they are host-specific.
- Validate after changing descriptors, schemas, policies, profiles, skills, graphs, patterns, or
  registry-generation code.

## Relationships

- [Panoply](../panoply/README.md) produces tool facts and is governed by Ontarch policies.
- [Takogami](../takogami/README.md) consumes trusted graph and bin contracts, applies runtime
  policy, and records command execution.
- [Polytope](../polytope/README.md) remains a planned package translator that will consume
  metadata-plane contracts.

## Related

- [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md) — detailed architecture
- [`../../docs/agent-configs.md`](../../docs/agent-configs.md) — profile and app integration
- [`../../docs/agent-rails.md`](../../docs/agent-rails.md) — policy and enforcement model
- [`../../docs/worker-guidance.md`](../../docs/worker-guidance.md) — repository-wide conventions

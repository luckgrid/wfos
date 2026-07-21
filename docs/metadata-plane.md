# Metadata plane — Ontarch 📐

The `metadata-plane` (Ontarch) stores the machine-readable meaning of the system:
**descriptors, registry, schemas, policies, graphs, models, and package contracts**. It exposes
no end-user runtime CLI — it is data and contracts that the other products read and write, plus
two build-time metadata tasks (`moon run ontarch:validate`, `moon run ontarch:sync`) that
validate those contracts and generate the registry from them.

Ontarch is delivered as a package (`packages/ontarch/`). It is the shared substrate the
[interface layers](architecture.md#interface-layers) sit on: profiles and policies decide how
much of the system each layer sees.

## Concepts

```txt
Descriptors   describe how things connect (paths, CLI, modules, runtime manager)
Registries    index what exists (tools, workspaces, apps, patterns, and their kinds)
Schemas       define contracts for generated data
Policies      define rules — including agent rails and gates
Graphs        define relationships — capability, policy, and unit dependency edges (generated)
Models        define machine-readable domain meaning (planned)
Packages      define package-translator-managed deliverable interfaces (planned)
```

## What lives here now

| Path | Kind | Purpose |
|------|------|---------|
| `descriptors/*.descriptor.toml` | descriptor | central unit descriptors (`panoply`, planned `ds`); colocated descriptors live beside their units (e.g. `wfos.descriptor.toml`) |
| `schemas/unit.schema.json` | schema | contract for unit descriptors (id, kind, paths, structured/legacy entrypoints, capabilities, policy) |
| `schemas/policy.schema.json` | schema | contract for policies (agent-rails + command styles) |
| `schemas/profile.schema.json` | schema | contract for agent operating profiles (scope, commands, validators, optional `[runtime] session_state_home`) |
| `schemas/skill.schema.json` | schema | contract for curated skill/template/pattern records (authored under `Workstreams/.agents/skills/`) |
| `schemas/command-output.schema.json` | schema | Takogami `--json` `CommandEnvelope` contract |
| `schemas/runtime-command-record.schema.json` | schema | operational Takogami command-execution record contract (distinct from build-session records) |
| `schemas/panoply.tools.schema.json` | schema | contract for the generated tools registry |
| `policies/panoply.agent.policy.toml` | policy | Panoply agent rails (allow/block, gates) |
| `policies/no-agent-git-push.policy.toml` | policy | agents never push or publish (human-only) |
| `graphs/edges.schema.json` | schema | contract for the project graph (nodes + directed edges) |
| `descriptors/takogami.descriptor.toml` | descriptor | runtime-controller unit (`kind = "runtime-controller"`) |
| `registry/{units,skills,profiles,policies,tools}.json` | registry | generated indexes (gitignored — host-specific) |
| `registry/graph.{json,dot}` | registry | generated project graph (gitignored — host-specific) |
| `registry/QUERIES.md`, `registry/queries/*.jq` | query | jq cookbook over the registry |
| `registry/sessions/*.json` | record | build-session records (tracked for provenance) |

The native toolchain produces `tools.json` (`panoply doctor`) and is governed by the agent
policy here; `ontarch sync` reads it and the descriptors/policies to emit the rest of the
registry.

## Generation and queries

`ontarch sync` walks descriptors (colocated beside units first; `descriptors/` is a central
override), policies, and **agent operating profiles** (`Workstreams/.agents/profiles/*.toml`),
and emits the registry as compact JSON. It also derives the project graph (`graph.json` +
`graph.dot`) from unit `capabilities`, policy `applies_to` edges, profile `selects` edges, and
profile `can-invoke` skill edges.
`ontarch validate` is the gate: it checks every descriptor, policy, **agent operating profile**
(`Workstreams/.agents/profiles/*.toml` vs `schemas/profile.schema.json`, including the
SkillSpector gate and `allowed_skill_ids` cross-ref), **curated skill records**
(`Workstreams/.agents/skills/*.toml` vs `schemas/skill.schema.json`, including the loadable-skill
scan gate), and the graph against its schema, reading the required keys and enums from
the schema itself so the schema stays the single source of truth. Both run on bash + `awk` + `jq`
(no new dependencies) and are agent-safe.

Generated `units.json` and `scan.json` include a `registry_generation` object
(`generated_at` + `source_fingerprints[]` over authored inputs). The runtime controller
(`takogami`) labels reads `hit` / `miss` / `stale` by recomputing those fingerprints; refresh
requires explicit consent (`takogami scan --refresh` → Ontarch scan/sync), never as a side
effect of a read-only query.

The registry is a **pre-computed context cache**. One filtered query answers what a repo scan
otherwise would:

```bash
jq -r --arg kind workspace -f registry/queries/by-kind.jq registry/units.json | jq -r .id
jq -r --arg cap proto       -f registry/queries/requires.jq registry/units.json
```

To learn what a workspace is, how to drive it, and the rails it runs under, an agent reads one
descriptor (or one filtered query) instead of scanning `moon.yml`, every package manifest, and
the READMEs to infer the same facts. Because the registry is generated and compact, it stays
cheaper to read than the source it summarizes. See
[`../packages/ontarch/registry/QUERIES.md`](../packages/ontarch/registry/QUERIES.md).

## Interface-layer exposure

The metadata plane materializes cross-layer contracts so each
[interface layer](architecture.md#interface-layers) sees the right amount:

```txt
Toolchain layer (low)     paths, native manifests, adapter contracts, registry scans
Agent layer   (mid)       descriptors, policies, scoped graphs, session context
Application layer (high)  workflow intent, domain/system labels — minimal path surface
```

## Stream metadata

Domain data, stream classification, and privacy policy are metadata-plane metadata — not a
folder. A stream's classification tier runs `private → internal → restricted → shared → public →
federated`, and promotion scope is the abstract **Leader** policy. Optional domain libraries
can appear on disk only when filesystem expression is actually needed.

## Relationships

- **`native-toolchain` (Panoply)** produces the registry (`panoply doctor`) and is governed by
  the agent policy here.
- **`runtime-controller` (Takogami, `takogami`)** is **in progress** — descriptor, schemas,
  Ontarch projection, and Rust contracts ship today; full discovery, routing, policy
  execution, and operational sessions are still ahead. **`package-translator` (Polytope,
  `takogami package`)** remains planned and will read metadata-plane data when implemented.
- **Native manifests stay authoritative.** The metadata plane describes meaning, routing,
  policy, and relationships; it does not replace `Cargo.toml`, `package.json`, `mise.toml`, or
  lockfiles.

## Adding metadata

Each product contributes its own descriptor, schema(s), and policy following the native-toolchain
example: a descriptor for how it connects, a schema for any generated artifact, and a policy
for its agent rails. Agent operating profiles are authored under `Workstreams/.agents/profiles/`
and validated/indexed by the metadata plane (see [agent-configs.md](agent-configs.md)). Generated,
host-specific output goes under `registry/` and is gitignored; contracts and policies are tracked.

See [native-toolchain.md](native-toolchain.md) for the producer side and [agent-rails.md](agent-rails.md) for how
policies are enforced.

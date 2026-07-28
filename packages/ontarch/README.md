# `metadata-plane` — Ontarch 📐

The metadata-plane (Ontarch) stores the machine-readable meaning of the system: **descriptors, registry, schemas,
policies, graphs, models, and package contracts**. It exposes no end-user runtime CLI — it is
data and contracts the other products read and write, plus build-time metadata tasks that
generate and validate the registry from those contracts.

Deep dive: [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md).

## Tasks

| Task | Purpose |
|------|---------|
| `moon run ontarch:validate` | gate — validate descriptors, policies, profiles, skills, graph, and bin contracts against their JSON schemas |
| `moon run ontarch:sync` | generate the registry (`units/skills/profiles/policies.json` + graph) |
| `moon run ontarch:scan` | emit the read-only polyrepo scan report |
| `moon run ontarch:bin-report` | emit report-only bin inventory (`bin-inventory.json` + `BIN-INVENTORY.md`) |
| `moon run ontarch:bin-cleanup` | cleanup plan modes (report-only / dry-run / deferred archive & delete-approved) |
| `moon run ontarch:agents-init` | seed a working `$AGENTS_HOME` (`.agents/`) from `patterns/agents/` |

All are dependency-free (bash + `awk` + `jq`), read-only over sources (except agents-init, which
writes only the navigation layer, and registry emitters that write under `registry/`), and
agent-safe where applicable.

## What lives here now

| Path | Kind | Purpose |
|------|------|---------|
| `descriptors/*.descriptor.toml` | descriptor | central unit descriptors (`panoply`, planned `ds`); colocated descriptors live beside their units (e.g. `wfos.descriptor.toml` at the workspace root) |
| `schemas/unit.schema.json` | schema | contract for unit descriptors (metadata-plane) |
| `schemas/policy.schema.json` | schema | contract for policies (agent-rails + command styles) |
| `schemas/profile.schema.json` | schema | contract for agent operating profiles |
| `schemas/command-output.schema.json` | schema | Takogami `--json` `CommandEnvelope` contract |
| `schemas/runtime-command-record.schema.json` | schema | operational Takogami command-execution record contract (distinct from build-session records) |
| `schemas/bin-inventory.schema.json` | schema | report-only bin inventory machine contract |
| `schemas/bin-cleanup-plan.schema.json` | schema | cleanup plan machine contract |
| `schemas/panoply.tools.schema.json` | schema | contract for the generated tools registry |
| `policies/panoply.agent.policy.toml` | policy | native-toolchain agent rails (allow/block, gates) |
| `policies/takogami.agent.policy.toml` | policy | runtime-controller request/child rails (incl. bin modes) |
| `policies/agent-bin.policy.toml` | policy | bin/archive allow/gate/block tiers |
| `policies/no-agent-git-push.policy.toml` | policy | agents never push or publish (human-only) |
| `graphs/edges.schema.json` | schema | contract for the project graph (metadata-plane graphs) |
| `lib/`, `bin/ontarch{,-sync,-validate,-scan,-bin-report,-bin-cleanup,-agents-init}` | code | registry generator + validator + scan/bin adapters + agents pattern seeder (bash/awk/jq) |
| `patterns/agents/` | pattern | agents navigation seed (contracts, templates, generic examples) — materialize with `agents-init` |
| `registry/QUERIES.md`, `registry/queries/*.jq` | query | the jq cookbook over the registry |
| `registry/{units,skills,profiles,policies,tools}.json` | registry | generated indexes (gitignored — host-specific) |
| `registry/graph.{json,dot}` | registry | generated project graph (gitignored — host-specific) |
| `registry/bin-inventory.json`, `BIN-INVENTORY.md` | registry | generated bin inventory (gitignored — host-specific) |
| `registry/sessions/*.json` | record | build-session provenance records (tracked; distinct from runtime command records) |
| `registry/.gitkeep` | — | keeps the registry directory tracked |

## Concepts

```txt
Descriptors  describe how things connect.
Registries   index what exists (tools, workspaces, apps, patterns, and their kinds).
Schemas      define contracts.
Policies     define rules — including agent rails and gates.
Graphs       define relationships — project deps + capability + policy edges.
Models       define machine-readable domain meaning (planned).
Packages     define package-translator (Polytope)-managed deliverable interfaces (planned).
```

## Relationships

- **[native-toolchain (Panoply)](../panoply/README.md)** produces the tools registry (`panoply doctor`) and is governed by the
  agent policy here. Today Panoply, Ontarch, and the **runtime-controller MVP (Takogami)** are the
  implemented Level 0 trio.
- **runtime-controller (Takogami)** (`takogami`) is the validated graph/bin consumer: discovery,
  dual-layer policy, direct `--execute`, command-execution sessions (`session list|show|latest`),
  `takogami graph`, and supported `takogami bin` projections ship as the E09 MVP. Ontarch remains
  the metadata owner and never becomes the runtime executor. Interactive providers and
  work-session restore remain post-MVP.
  **package-translator (Polytope)** (`takogami package`) remains planned and will read metadata-plane
  data when implemented.
- **Native manifests stay authoritative** — the metadata-plane describes meaning, routing, policy, and
  relationships; it does not replace `Cargo.toml`, `package.json`, `mise.toml`, or lockfiles.

## Interface-layer exposure

```txt
Toolchain layer (low)     paths, native manifests, adapter contracts, registry scans
Agent layer   (mid)       descriptors, policies, scoped graphs, session context
Application layer (high)  workflow intent, domain/system labels — minimal path surface
```

## Related

- [`AGENTS.md`](AGENTS.md) — agent rules for editing metadata
- [`../panoply/README.md`](../panoply/README.md) — the producer/consumer of this metadata
- [`../takogami/README.md`](../takogami/README.md) — runtime-controller MVP consumer
- [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md) · [`../../docs/agent-rails.md`](../../docs/agent-rails.md)

# `metadata-plane` — Ontarch 📐

The metadata-plane (Ontarch) stores the machine-readable meaning of the system: **descriptors, registry, schemas,
policies, graphs, models, and package contracts**. It exposes no end-user runtime CLI — it is
data and contracts the other products read and write, plus two build-time metadata tasks that
generate and validate the registry from those contracts.

Deep dive: [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md).

## Tasks

| Task | Purpose |
|------|---------|
| `moon run ontarch:validate` | gate — validate descriptors, policies, and the generated graph against their JSON schemas |
| `moon run ontarch:sync` | generate the registry (`units/skills/profiles/policies.json` + graph) |

Both are dependency-free (bash + `awk` + `jq`), read-only over sources, and write only
generated output under `registry/` — agent-safe.

## What lives here now

| Path | Kind | Purpose |
|------|------|---------|
| `descriptors/*.descriptor.toml` | descriptor | central unit descriptors (`panoply`, planned `ds`); colocated descriptors live beside their units (e.g. `wfos.descriptor.toml` at the workspace root) |
| `schemas/unit.schema.json` | schema | contract for unit descriptors (metadata-plane) |
| `schemas/policy.schema.json` | schema | contract for policies (agent-rails + command styles) |
| `schemas/panoply.tools.schema.json` | schema | contract for the generated tools registry |
| `policies/panoply.agent.policy.toml` | policy | native-toolchain agent rails (allow/block, gates) |
| `policies/no-agent-git-push.policy.toml` | policy | agents never push or publish (human-only) |
| `graphs/edges.schema.json` | schema | contract for the project graph (metadata-plane graphs) |
| `lib/`, `bin/ontarch{,-sync,-validate}` | code | the registry generator + validator (bash/awk/jq) |
| `registry/QUERIES.md`, `registry/queries/*.jq` | query | the jq cookbook over the registry |
| `registry/{units,skills,profiles,policies,tools}.json` | registry | generated indexes (gitignored — host-specific) |
| `registry/graph.{json,dot}` | registry | generated project graph (gitignored — host-specific) |
| `registry/sessions/*.json` | record | build-session records (tracked for provenance) |
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

- **[native-toolchain (Panoply)](../panoply/README.md)** produces the registry (`panoply doctor`) and is governed by the
  agent policy here. Today the metadata-plane + native-toolchain are the implemented pair.
- **runtime-controller (Takogami)** (`takogami`) is **in progress** — descriptor, schemas,
  projection, and Rust contracts ship today; discovery/routing/sessions are still ahead.
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
- [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md) · [`../../docs/agent-rails.md`](../../docs/agent-rails.md)

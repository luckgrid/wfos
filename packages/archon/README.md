# Archon — metadata plane

Archon stores the machine-readable meaning of the system: **descriptors, registry, schemas,
policies, graphs, models, and package contracts**. It has no CLI — it is data and contracts
the other products read and write.

Deep dive: [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md).

## What lives here now

| Path | Kind | Purpose |
|------|------|---------|
| `descriptors/dust.descriptor.toml` | descriptor | how Dust connects — paths, CLI, modules, runtime manager |
| `schemas/dust.tools.schema.json` | schema | contract for the generated tools registry |
| `policies/dust.agent.policy.toml` | policy | Dust agent rails (allow/block, gates) |
| `registry/tools.json` | registry | tool inventory from `dust doctor` (gitignored — host-specific) |
| `registry/.gitkeep` | — | keeps the registry directory tracked |

## Concepts

```txt
Descriptors  describe how things connect.
Registries   index what exists (tools, workspaces, apps, patterns, and their kinds).
Schemas      define contracts.
Policies     define rules — including agent rails and gates.
Graphs       define relationships — project deps + git resources (planned).
Models       define machine-readable domain meaning (planned).
Packages     define Hypercube-managed deliverable interfaces (planned).
```

## Relationships

- **[Dust](../dust/README.md)** produces the registry (`dust doctor`) and is governed by the
  agent policy here. Today Archon + Dust are the implemented pair.
- **Kraken** (`krk`) and **Hypercube** (`hqb`) will read and operate on Archon metadata when
  implemented.
- **Native manifests stay authoritative** — Archon describes meaning, routing, policy, and
  relationships; it does not replace `Cargo.toml`, `package.json`, `mise.toml`, or lockfiles.

## Interface-layer exposure

```txt
Toolchain layer (low)     paths, native manifests, adapter contracts, registry scans
Agent layer   (mid)       descriptors, policies, scoped graphs, session context
Application layer (high)  workflow intent, domain/system labels — minimal path surface
```

## Related

- [`AGENTS.md`](AGENTS.md) — agent rules for editing metadata
- [`../dust/README.md`](../dust/README.md) — the producer/consumer of this metadata
- [`../../docs/metadata-plane.md`](../../docs/metadata-plane.md) · [`../../docs/agent-rails.md`](../../docs/agent-rails.md)

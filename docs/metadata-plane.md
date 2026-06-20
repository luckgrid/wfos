# Metadata plane — Archon

Archon stores the machine-readable meaning of the system: **descriptors, registry, schemas,
policies, graphs, models, and package contracts**. It has no CLI of its own — it is data and
contracts that the other products read and write.

Archon is delivered as a package (`packages/archon/`). It is the shared substrate the
[interface layers](architecture.md#interface-layers) sit on: profiles and policies decide how
much of the system each layer sees.

## Concepts

```txt
Descriptors   describe how things connect (paths, CLI, modules, runtime manager)
Registries    index what exists (tools, workspaces, apps, patterns, and their kinds)
Schemas       define contracts for generated data
Policies      define rules — including agent rails and gates
Graphs        define relationships — project deps + git resources (planned)
Models        define machine-readable domain meaning (planned)
Packages      define Hypercube-managed deliverable interfaces (planned)
```

## What lives here now

| Path | Kind | Purpose |
|------|------|---------|
| `descriptors/dust.descriptor.toml` | descriptor | how Dust connects — paths, CLI, modules, runtime manager |
| `schemas/dust.tools.schema.json` | schema | contract for the generated tools registry |
| `policies/dust.agent.policy.toml` | policy | Dust agent rails (allow/block, gates) |
| `registry/tools.json` | registry | tool inventory produced by `dust doctor` (gitignored — host-specific) |
| `registry/.gitkeep` | — | keeps the registry directory tracked |

Today Archon + Dust are the implemented pair: Dust produces the registry and is governed by
the agent policy here.

## Interface-layer exposure

Archon materializes cross-layer contracts so each [interface layer](architecture.md#interface-layers)
sees the right amount:

```txt
Toolchain layer (low)     paths, native manifests, adapter contracts, registry scans
Agent layer   (mid)       descriptors, policies, scoped graphs, session context
Application layer (high)  workflow intent, domain/system labels — minimal path surface
```

## Stream metadata

Domain data, stream classification, and privacy policy are Archon metadata — not a folder.
A stream's classification tier runs `private → internal → restricted → shared → public →
federated`, and promotion scope is the abstract **Leader** policy. Optional domain libraries
can appear on disk only when filesystem expression is actually needed.

## Relationships

- **Dust** (native substrate) produces the registry (`dust doctor`) and is governed by the
  agent policy here.
- **Kraken** (`krk`) and **Hypercube** (`hqb`) will read and operate on Archon metadata when
  implemented — discovery, routing, sessions, and package translation.
- **Native manifests stay authoritative.** Archon describes meaning, routing, policy, and
  relationships; it does not replace `Cargo.toml`, `package.json`, `mise.toml`, or lockfiles.

## Adding metadata

Each product contributes its own descriptor, schema(s), and policy following the Dust
example: a descriptor for how it connects, a schema for any generated artifact, and a policy
for its agent rails. Generated, host-specific output goes under `registry/` and is gitignored;
contracts and policies are tracked.

See [native-substrate.md](native-substrate.md) for the producer side and [agent-rails.md](agent-rails.md) for how
policies are enforced.

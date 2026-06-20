# WfOS documentation

WfOS is a **local-first Workflows Operating System** for a developer machine: a thin
control layer over the tools you already use. It is modular and non-disruptive — adopt
one package, keep your own shell and editor, and grow into the rest when it earns its place.

This folder is the self-contained reference for the workspace. Start with
[architecture](architecture.md), then read the guide for whichever product you are touching.

## Reference matrix

| Doc | What it covers |
|-----|----------------|
| [architecture.md](architecture.md) | Archetypes vs products, interface layers, the system map |
| [runtime-architecture.md](runtime-architecture.md) | Terminal-first engine: client-daemon model and the Rust stack |
| [monorepo.md](monorepo.md) | moon project graph + proto toolchains, tasks, conventions |
| [native-substrate.md](native-substrate.md) | Native substrate — Unix/Rust tools, modules, config templates |
| [metadata-plane.md](metadata-plane.md) | Metadata plane — descriptors, registry, schemas, policies |
| [runtime-controller.md](runtime-controller.md) | Runtime controller (`krk`) — discovery, routing, sessions (planned) |
| [package-translator.md](package-translator.md) | Package translator (`hqb`) — intent → packages (planned) |
| [portable-runtime.md](portable-runtime.md) | Portable runtime — WASM/WASI components (planned) |
| [agent-rails.md](agent-rails.md) | Agent rails, gates, MCP exposure, skill scanning |
| [apps.md](apps.md) | Docs site + marketing site (Zola) |
| [tool-catalog.md](tool-catalog.md) | Grouped catalog of tools, libraries, skills, and crates |
| [workflow-apps.md](workflow-apps.md) | Core native workflow apps — notes, writing, AI engine, sessions |
| [setup.md](setup.md) | Setup for developers and agents |
| Workstreams canon (external) | [Plan/bin/lg_wfos_ws_namespaces.md](../../../../../../Plan/bin/lg_wfos_ws_namespaces.md) — filesystem namespaces |

## The five products

| Archetype | Product | CLI | Role |
|-----------|---------|-----|------|
| runtime-controller | Kraken | `krk` | Discovery, routing, sessions, agent rails |
| package-translator | Hypercube | `hqb` | Intent → packages and artifacts |
| native-substrate | Dust | `dust` | Local Unix/Rust tool execution |
| portable-runtime | Ether | — | WASM/WASI sandboxed components |
| metadata-plane | Archon | — | Descriptors, registry, schemas, policies |

Archetypes are stable roles; products are the implementations shipped here. Any product is
swappable — the archetype id is what other layers depend on.

## Status

Implemented today: **Dust** (native substrate) and **Archon** (metadata plane).
Planned: **Kraken**, **Hypercube**, **Ether**. See each guide for scope and roadmap.

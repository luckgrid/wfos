# WfOS documentation

WfOS is a **local-first Workflows Operating System** for a developer machine: a thin
control layer over the tools you already use. It is modular and non-disruptive — adopt
one package, keep your own shell and editor, and grow into the rest when it earns its place.

This folder is the self-contained reference for the workspace. Start with
[architecture](architecture.md), then read the guide for whichever archetype or product you are touching.

## Reference matrix

| Doc | What it covers |
|-----|----------------|
| [architecture.md](architecture.md) | Archetypes vs products, interface layers, the system map |
| [runtime-architecture.md](runtime-architecture.md) | Engine blueprint: v0 CLI first; daemon/TUI optional and not a terminal multiplexer |
| [monorepo.md](monorepo.md) | moon project graph + proto toolchains, tasks, conventions |
| [native-toolchain.md](native-toolchain.md) | Native toolchain — Unix/Rust tools, modules, config templates |
| [metadata-plane.md](metadata-plane.md) | Metadata plane — descriptors, registry, schemas, policies |
| [runtime-controller.md](runtime-controller.md) | Runtime controller (`takogami`) — MVP discovery/routing/command records; providers for terminals |
| [package-translator.md](package-translator.md) | Package translator (`takogami package`) — intent → packages (planned) |
| [portable-component-runtime.md](portable-component-runtime.md) | Portable component runtime — WASM/WASI components (planned) |
| [agent-configs.md](agent-configs.md) | Shared agent profiles, app-integration pattern, lean AGENTS.md |
| [agent-skills.md](agent-skills.md) | On-demand skill registry, templates, Fabric patterns, load logging |
| [agent-rails.md](agent-rails.md) | Agent rails, gates, MCP exposure, skill scanning |
| [git-hygiene.md](git-hygiene.md) | Repo-local hooks, gitleaks gate, conventional commits |
| [bin-archive.md](bin-archive.md) | Bin inventory, manifests, cleanup modes, archive taxonomy |
| [apps.md](apps.md) | Docs site + marketing site (Zola) |
| [tool-catalog.md](tool-catalog.md) | Grouped catalog of tools, libraries, skills, and crates (incl. optional Herdr / desktop providers) |
| [workflow-apps.md](workflow-apps.md) | Core native workflow apps — notes, writing, AI engine, sessions |
| [setup.md](setup.md) | Setup for developers and agents |
| Workstreams namespaces | [architecture.md#workstreams-collection](architecture.md#workstreams-collection) — Plan, Brand, Build, Control |

## The five products

| Archetype | Product | CLI | Role |
|-----------|---------|-----|------|
| `runtime-controller` | Takogami 🐙 | `takogami` | Discovery, routing, command records, agent rails |
| `package-translator` | Polytope 📦 | `takogami package` | Intent → packages and artifacts |
| `native-toolchain` | Panoply 🧰 | `panoply` | Local Unix/Rust tool execution |
| `portable-component-runtime` | Wisp 🫧 | `takogami portable` (planned) | WASM/WASI sandboxed components |
| `metadata-plane` | Ontarch 📐 | — (build tasks; later `takogami meta`) | Descriptors, registry, schemas, policies |

Archetypes are stable roles; products are the implementations shipped here. Any product is
swappable — the archetype id is what other layers depend on.

## Status

Implemented today: **`native-toolchain` (Panoply)** and **`metadata-plane` (Ontarch)**.
In progress: **`runtime-controller` (Takogami)**.
Planned: **`package-translator` (Polytope)**, **`portable-component-runtime` (Wisp)**.
See each guide for scope and roadmap; build-session provenance lives in
`packages/ontarch/registry/sessions/`.

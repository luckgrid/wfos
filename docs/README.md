# WfOS documentation

Self-contained reference for this workspace. Begin at the root [`README.md`](../README.md), then
use this page to enter the detailed guide for the package or concern being changed.

Package inventory and status live in the [workspace README](../README.md#package-map). Install and
bootstrap guidance lives in [setup.md](setup.md). Do not duplicate those sources here.

## Guides

| Doc | Covers |
|---|---|
| [architecture.md](architecture.md) | Archetypes vs products, interface layers, and system map |
| [runtime-architecture.md](runtime-architecture.md) | Engine blueprint: v0 CLI first; daemon and TUI optional |
| [monorepo.md](monorepo.md) | Moon project graph, proto pins, tasks, and conventions |
| [setup.md](setup.md) | Developer and automated-worker setup |
| [worker-guidance.md](worker-guidance.md) | README-first entrypoints, authority order, and repository-wide worker conventions |
| [native-toolchain.md](native-toolchain.md) | Panoply tools, modules, configuration, and substrate boundary |
| [metadata-plane.md](metadata-plane.md) | Ontarch descriptors, registry, schemas, policies, profiles, and patterns |
| [runtime-controller.md](runtime-controller.md) | Takogami discovery, routing, execution, graph/bin, and command records |
| [package-translator.md](package-translator.md) | Polytope (`takogami package`) — planned |
| [portable-component-runtime.md](portable-component-runtime.md) | Wisp — planned |
| [agent-configs.md](agent-configs.md) | Shared agent profiles, app integration, and README-first instruction placement |
| [agent-skills.md](agent-skills.md) | On-demand skill registry, templates, scanning, and load logging |
| [agent-rails.md](agent-rails.md) | Rails, gates, MCP, isolation, and skill scanning |
| [git-hygiene.md](git-hygiene.md) | Hooks, gitleaks, and conventional commits |
| [bin-archive.md](bin-archive.md) | Bin inventory, manifests, and cleanup modes |
| [apps.md](apps.md) | Documentation and marketing applications |
| [tool-catalog.md](tool-catalog.md) | Tools, libraries, skills, and crates |
| [workflow-apps.md](workflow-apps.md) | Notes, writing, AI engine, and sessions |
| Workstreams | [architecture § Workstreams](architecture.md#workstreams-collection) |

Build-session provenance is tracked under `packages/ontarch/registry/sessions/`.

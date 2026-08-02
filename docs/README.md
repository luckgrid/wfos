# WfOS documentation

Self-contained reference for this workspace. Start with [architecture](architecture.md),
then the guide for the package you are touching.

**Package inventory and status** live in the [workspace README](../README.md#packages) —
do not duplicate them here. **Install / bootstrap** lives in [setup.md](setup.md).

## Guides

| Doc | Covers |
|-----|--------|
| [architecture.md](architecture.md) | Archetypes vs products, interface layers, system map |
| [runtime-architecture.md](runtime-architecture.md) | Engine blueprint: v0 CLI first; daemon/TUI optional |
| [monorepo.md](monorepo.md) | moon project graph + proto pins, tasks, conventions |
| [setup.md](setup.md) | Developer and agent setup |
| [native-toolchain.md](native-toolchain.md) | Panoply — tools, modules, config |
| [metadata-plane.md](metadata-plane.md) | Ontarch — descriptors, registry, schemas, policies |
| [runtime-controller.md](runtime-controller.md) | Takogami MVP — discovery, routing, execute, graph/bin, command records |
| [package-translator.md](package-translator.md) | Polytope (`takogami package`) — planned |
| [portable-component-runtime.md](portable-component-runtime.md) | Wisp — planned |
| [agent-configs.md](agent-configs.md) | Shared agent profiles and README entrypoints |
| [agent-skills.md](agent-skills.md) | On-demand skill registry, templates, load logging |
| [agent-rails.md](agent-rails.md) | Rails, gates, MCP, skill scanning |
| [git-hygiene.md](git-hygiene.md) | Hooks, gitleaks, conventional commits |
| [bin-archive.md](bin-archive.md) | Bin inventory, manifests, cleanup modes |
| [apps.md](apps.md) | Docs + marketing sites |
| [tool-catalog.md](tool-catalog.md) | Tools, libraries, skills, crates |
| [workflow-apps.md](workflow-apps.md) | Notes, writing, AI engine, sessions |
| Workstreams | [architecture § Workstreams](architecture.md#workstreams-collection) |

Build-session provenance: `packages/ontarch/registry/sessions/`.

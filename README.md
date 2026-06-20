# WfOS

**WfOS** (Workflows Operating System) is a local-first control layer for a developer machine.
It does not replace your OS, shell, or package managers — it organizes them, routes to them,
and exposes their meaning through a consistent local interface. It is modular and
non-disruptive: adopt one package, keep your own environment, and grow into the rest when it
earns its place.

Full documentation lives in [`docs/`](docs/README.md). Start with
[architecture](docs/architecture.md).

## Products

| Archetype | Product | CLI | Role | Status |
|-----------|---------|-----|------|--------|
| native-substrate | Dust | `dust` | Local Unix/Rust tool execution | implemented |
| metadata-plane | Archon | — | Descriptors, registry, schemas, policies | implemented |
| runtime-controller | Kraken | `krk` | Discovery, routing, sessions, rails | planned |
| package-translator | Hypercube | `hqb` | Intent → packages and artifacts | planned |
| portable-runtime | Ether | — | WASM/WASI sandboxed components | planned |

Archetypes are stable roles; products are swappable implementations. Above the filesystem,
three [interface layers](docs/architecture.md#interface-layers) — toolchain, agent, application
— expose the system at the depth that matches how you work.

## Monorepo & toolchain

This workspace is a [moon](https://moonrepo.dev/moon) monorepo with toolchains pinned by
[proto](https://moonrepo.dev/proto). Install proto + moon, then:

```bash
moon run wfos:setup     # proto install — fetch pinned toolchains
moon run dust:doctor    # detect tools + write the Archon registry (read-only)
moon query projects     # inspect the project graph
```

Pins live in [`.prototools`](.prototools); graph and tasks in [`.moon/`](.moon/) and per-project
`moon.yml`. See [docs/monorepo.md](docs/monorepo.md) and [docs/setup.md](docs/setup.md).

## Packages

| Package | Role | Status |
|---------|------|--------|
| [`archon/`](packages/archon/README.md) | metadata plane — descriptors, schemas, policies | implemented |
| [`dust/`](packages/dust/README.md) | native substrate — global low-level tools | implemented |
| [`ether/`](packages/ether/README.md) | portable runtime (WASM/WASI) | planned |
| [`hypercube/`](packages/hypercube/README.md) | package translator (`hqb`) | planned |
| [`kraken/`](packages/kraken/README.md) | runtime controller (`krk`) | planned |

## Apps

| App | Purpose | Status |
|-----|---------|--------|
| [`apps/docs/`](apps/docs/README.md) | render the docs for humans (Zola) | planned |
| [`apps/web/`](apps/web/README.md) | single-page marketing site (Zola) | planned |

## Documentation matrix

| Doc | Covers |
|-----|--------|
| [architecture](docs/architecture.md) | Archetypes vs products, interface layers, system map |
| [runtime-architecture](docs/runtime-architecture.md) | Terminal-first engine: client-daemon, Rust stack |
| [monorepo](docs/monorepo.md) | moon + proto graph, tasks, conventions |
| [native-substrate](docs/native-substrate.md) | Native substrate — tools, modules, config |
| [metadata-plane](docs/metadata-plane.md) | Metadata plane — descriptors, registry, schemas, policies |
| [runtime-controller](docs/runtime-controller.md) · [package-translator](docs/package-translator.md) · [portable-runtime](docs/portable-runtime.md) | Planned products |
| [agent-rails](docs/agent-rails.md) | Agent rails, gates, MCP, skill scanning |
| [apps](docs/apps.md) | Docs + marketing sites |
| [tool-catalog](docs/tool-catalog.md) | Grouped tools, libraries, skills, crates |
| [workflow-apps](docs/workflow-apps.md) | Core native workflow apps — notes, writing, AI engine, sessions |
| [setup](docs/setup.md) | Developer and agent setup |

For agents, see [AGENTS.md](AGENTS.md).

## Git

This workspace is its own standalone git repository (`main`), local-first with no required
remote. Generated, host-specific output (the Archon tools registry, `target/`, `.moon/cache`)
is gitignored; sources, contracts, and docs stay tracked.

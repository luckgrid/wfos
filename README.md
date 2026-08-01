# WfOS

**WfOS** (Workflows Operating System) is a local-first control layer for a developer machine.
It does not replace the operating system, shell, package managers, native manifests, or the
operator's own workspace model. It discovers, describes, routes, governs, and records how those
parts work together.

WfOS is modular and non-disruptive: adopt one package, keep the environment you already use, and
add more only when the capability earns its place.

## Start here

This `README.md` is the canonical entrypoint for **every worker**, human or automated.

1. Read this file for the workspace boundary, package map, and first commands.
2. Enter a package, app, or documentation namespace through its nearest `README.md`.
3. Run commands from the workspace root unless the local README says otherwise.
4. Treat native manifests as authoritative; Ontarch describes meaning, routing, and policy but
   does not replace them.
5. Automated workers must also obey the selected profile and WfOS policies described in
   [worker guidance](docs/worker-guidance.md), [agent configs](docs/agent-configs.md), and
   [agent rails](docs/agent-rails.md).

Repository-wide or package-level `AGENTS.md` files are not used. Shared instructions belong in
README files and normal documentation. Agent-specific scope, commands, secrets, validators, and
isolation belong in profiles, policies, schemas, and the reusable agents-navigation pattern.

## Quick start

Experienced-developer path from the workspace root:

```bash
curl -fsSL https://moonrepo.dev/install/proto.sh | bash   # once, if needed
proto install                                             # .prototools pins moon + rust
moon run wfos:setup
moon run panoply:doctor
moon run ontarch:sync
cargo build -p takogami

cargo run -p takogami -- doctor
cargo run -p takogami -- graph
cargo run -p takogami -- bin report
```

This is a [moon](https://moonrepo.dev/moon) + [proto](https://moonrepo.dev/proto) monorepo.
Detailed setup: [`docs/setup.md`](docs/setup.md). Project conventions:
[`docs/monorepo.md`](docs/monorepo.md).

## Package map

Archetypes are stable system roles. Product names are swappable implementations.

| Package | Archetype | Product | CLI | Role | Status |
|---|---|---|---|---|---|
| [`packages/panoply/`](packages/panoply/README.md) | `native-toolchain` | Panoply 🧰 | `panoply` | Local Unix/Rust tool substrate | implemented |
| [`packages/ontarch/`](packages/ontarch/README.md) | `metadata-plane` | Ontarch 📐 | build tasks (`ontarch:*`) | Descriptors, registry, schemas, policies, and graphs | implemented |
| [`packages/takogami/`](packages/takogami/README.md) | `runtime-controller` | Takogami 🐙 | `takogami` | Discovery, routing, policy, execution, graph/bin, and command records | implemented |
| [`packages/polytope/`](packages/polytope/README.md) | `package-translator` | Polytope 📦 | `takogami package` | Intent to packages and artifacts | planned |
| [`packages/wisp/`](packages/wisp/README.md) | `portable-component-runtime` | Wisp 🫧 | `takogami portable` | WASM/WASI sandboxed components | planned |

## App map

| App | Role | Status |
|---|---|---|
| [`apps/docs/`](apps/docs/README.md) | Render workspace documentation for humans | planned |
| [`apps/web/`](apps/web/README.md) | Single-page public site | planned |

## Architecture in one view

```text
native-toolchain
  provides local tools and environment facts
        ↓
metadata-plane
  describes units, contracts, policies, profiles, and relationships
        ↓
runtime-controller
  discovers, explains, routes, executes, and records bounded commands
        ↓
optional higher-level packages and applications
```

The layers remain replaceable and independently useful. A native manifest, package manager,
workspace README, or operator-owned filesystem remains authoritative for the concern it owns.

Start the deeper architecture at [`docs/architecture.md`](docs/architecture.md).

## Worker and documentation model

Each maintained namespace uses its README as a concise entrypoint:

```text
namespace/
├── README.md       purpose, map, first commands, and authority links
├── docs/           detailed explanation and runbooks when needed
├── native files    manifests, source, schemas, policies, tests, and configuration
└── child/
    └── README.md   child namespace entrypoint
```

Detailed automated-worker behavior is intentionally separate from the README layer:

| Concern | Authority |
|---|---|
| Workspace and namespace orientation | nearest `README.md` |
| Architecture and commands | package docs and native manifests |
| Agent session scope and allowed paths | selected `.agents/profiles/*.toml` |
| Command, secret, and mutation rails | Ontarch policies and runtime enforcement |
| Reusable agent navigation contracts | `packages/ontarch/patterns/agents/` |
| Generated unit/profile/policy projections | Ontarch registry and graph |
| Exact accepted history | Git commits and pull requests |

## Documentation

- [`docs/README.md`](docs/README.md) — documentation map
- [`docs/architecture.md`](docs/architecture.md) — archetypes, products, interfaces, and system map
- [`docs/worker-guidance.md`](docs/worker-guidance.md) — repository-wide worker conventions
- [`docs/agent-configs.md`](docs/agent-configs.md) — shared automated-worker profiles and app integration
- [`docs/agent-rails.md`](docs/agent-rails.md) — policies, gates, isolation, and enforcement boundaries
- [`docs/setup.md`](docs/setup.md) — local setup
- [`docs/monorepo.md`](docs/monorepo.md) — moon graph, proto pins, tasks, and conventions

## Repository behavior

WfOS is a standalone Git repository on `main`. Generated host output—including the Ontarch
registry, build targets, and Moon cache—is gitignored. Authored source, contracts, tests, and
documentation remain tracked.

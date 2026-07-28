# WfOS

**WfOS** (Workflows Operating System) is a local-first control layer for a developer machine.
It does not replace your OS, shell, or package managers — it organizes them, routes to them,
and exposes their meaning through a consistent local interface. It is modular and
non-disruptive: adopt one package, keep your own environment, and grow into the rest when it
earns its place.

Full docs: [`docs/`](docs/README.md). Start with [architecture](docs/architecture.md).
Archetypes are stable roles; products are swappable implementations — see
[architecture § archetypes](docs/architecture.md#archetypes-vs-products).

## Quick start

Experienced-dev path (Rust + this workspace root):

```bash
curl -fsSL https://moonrepo.dev/install/proto.sh | bash   # once, if needed
proto install                                             # .prototools pins (moon + rust)
moon run wfos:setup
moon run panoply:doctor
moon run ontarch:sync
cargo build -p takogami

cargo run -p takogami -- doctor
cargo run -p takogami -- graph
cargo run -p takogami -- bin report
```

`moon run takogami:build` / `takogami:test` work the same. This is a
[moon](https://moonrepo.dev/moon) + [proto](https://moonrepo.dev/proto) monorepo
([`.prototools`](.prototools), [`.moon/`](.moon/)). Details:
[docs/setup.md](docs/setup.md) · [docs/monorepo.md](docs/monorepo.md).

## Packages

| Package | Archetype | Product | CLI | Role | Status |
|---------|-----------|---------|-----|------|--------|
| [`panoply/`](packages/panoply/README.md) | `native-toolchain` | Panoply 🧰 | `panoply` (later `takogami native`) | Local Unix/Rust tool execution | implemented |
| [`ontarch/`](packages/ontarch/README.md) | `metadata-plane` | Ontarch 📐 | build tasks (`ontarch:*`; later `takogami meta`) | Descriptors, registry, schemas, policies | implemented |
| [`takogami/`](packages/takogami/README.md) | `runtime-controller` | Takogami 🐙 | `takogami` | Discovery, routing, policy, execute, graph/bin, command records | implemented |
| [`polytope/`](packages/polytope/README.md) | `package-translator` | Polytope 📦 | `takogami package` | Intent → packages and artifacts | planned |
| [`wisp/`](packages/wisp/README.md) | `portable-component-runtime` | Wisp 🫧 | `takogami portable` (planned) | WASM/WASI sandboxed components | planned |

## Apps

| App | Role | Status |
|-----|------|--------|
| [`apps/docs/`](apps/docs/README.md) | Render workspace docs for humans (Zola) | planned |
| [`apps/web/`](apps/web/README.md) | Single-page marketing site (Zola) | planned |

## Docs

Index and guides: [`docs/README.md`](docs/README.md). Agents: [AGENTS.md](AGENTS.md).

## Git

Standalone git repository (`main`), local-first. Generated host output (metadata-plane
registry, `target/`, `.moon/cache`) is gitignored; sources, contracts, and docs stay tracked.

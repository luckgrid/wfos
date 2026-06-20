# Monorepo: moon + proto

The workspace is a [moon](https://moonrepo.dev/moon) monorepo with toolchains pinned by
[proto](https://moonrepo.dev/proto). The split is deliberate and simple:

- **proto** pins and installs language runtimes and tools (`.prototools`). No global
  installs beyond proto itself.
- **moon** owns the project graph, task running, caching, and affected-project detection
  (`.moon/`, per-project `moon.yml`).

This keeps a fresh clone reproducible and lets both humans and agents run the same tasks the
same way.

## Layout

```txt
.prototools              toolchain pins (proto, moon, rust; node/zola added with the apps)
.moon/
  workspace.yml          project graph (sources) + VCS settings
  toolchains.yml         enabled toolchains (rust today)
moon.yml                 root project "wfos" — setup + aggregate tasks
packages/<name>/moon.yml per-project config and tasks
```

`.moon/cache` and `target/` are ignored; everything else is tracked.

## Project graph

Projects are declared explicitly in `.moon/workspace.yml` while packages and apps are still
being scaffolded:

```yaml
projects:
  sources:
    wfos: "."
    dust: "packages/dust"
    archon: "packages/archon"
```

As `packages/kraken`, `packages/hypercube`, `packages/ether`, and `apps/*` gain their own
`moon.yml`, add them here (or switch to `apps/*` / `packages/*` globs once every directory
under those paths is a real project). Keeping it explicit avoids moon trying to treat a stub
folder as a project before it has any config.

## Tasks today

```bash
moon run wfos:setup     # proto install — fetch pinned toolchains
moon run dust:doctor    # detect tools + write the Archon registry (read-only)
moon run dust:list      # list Dust modules and tools
moon run dust:env       # print the shell activation snippet
moon run wfos:doctor    # aggregate: depends on dust:doctor
moon query projects     # inspect the graph
```

The `dust:*` tasks are thin wrappers over `packages/dust/bin/dust` so the substrate joins
the graph. They are `cache: false` because `doctor` inspects the live machine and `bootstrap`
is intentionally not exposed as a task (it is human-only — see [agent-rails.md](agent-rails.md)).

## Conventions for new projects

- **Apps** (`apps/*`): each app owns its ports, env, and dev/build commands; add
  orchestration to the root only when more than one app benefits.
- **Packages** (`packages/*`): shared infrastructure with stable, composable interfaces.
- Each new project gets a `moon.yml` declaring `layer`, `language`, and tasks. Rust crates
  follow the standard `build` / `test` / `lint` (`cargo clippy -D warnings`) /
  `format-check` (`cargo fmt --check`) task set.

## Rust workspace (deferred)

There are no Rust crates yet, so there is no root `Cargo.toml`. The Cargo workspace lands
with the first crate ([Kraken](runtime-controller.md)); at that point the root `Cargo.toml` lists
`packages/kraken` as a member, and `.moon/toolchains.yml` already has the rust toolchain
enabled with `clippy` and `rustfmt`.

## How Kraken relates to moon

[Kraken](runtime-controller.md) (`krk`) is the WfOS runtime controller, not a replacement for moon. moon
is the project-graph and task backend; Kraken routes higher-level workflow commands to moon's
task graph (a compat backend) and to [Dust](native-substrate.md) native tools, recording sessions and
applying policy along the way. Native manifests and the moon graph stay authoritative for
builds; Kraken adds discovery, routing, and rails on top.

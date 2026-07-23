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
    panoply: "packages/panoply"
    ontarch: "packages/ontarch"
```

As `packages/takogami`, `packages/polytope`, `packages/wisp`, and `apps/*` gain their own
`moon.yml`, add them here (or switch to `apps/*` / `packages/*` globs once every directory
under those paths is a real project). Keeping it explicit avoids moon trying to treat a stub
folder as a project before it has any config.

## Tasks today

```bash
moon run wfos:setup     # proto install — fetch pinned toolchains
moon run panoply:doctor    # detect tools + write the Ontarch registry (read-only)
moon run panoply:list      # list Panoply modules and tools
moon run panoply:env       # print the shell activation snippet
moon run wfos:doctor    # aggregate: depends on panoply:doctor
moon query projects     # inspect the graph
```

The `panoply:*` tasks are thin wrappers over `packages/panoply/bin/panoply` so the substrate joins
the graph. They are `cache: false` because `doctor` inspects the live machine and `bootstrap`
is intentionally not exposed as a task (it is human-only — see [agent-rails.md](agent-rails.md)).

## Conventions for new projects

- **Apps** (`apps/*`): each app owns its ports, env, and dev/build commands; add
  orchestration to the root only when more than one app benefits.
- **Packages** (`packages/*`): shared infrastructure with stable, composable interfaces.
- Each new project gets a `moon.yml` declaring `layer`, `language`, and tasks. Rust crates
  follow the standard `build` / `test` / `lint` (`cargo clippy -D warnings`) /
  `format-check` (`cargo fmt --check`) task set.

## Rust workspace

The Cargo workspace landed with the runtime-controller foundation. Root `Cargo.toml` lists `packages/takogami` as the
member crate; `Cargo.lock` is committed. `.moon/toolchains.yml` already enables the rust
toolchain with `clippy` and `rustfmt`. See the [runtime controller (Takogami)](runtime-controller.md)
guide and [`packages/takogami/README.md`](../packages/takogami/README.md) for the current
proved surface (discovery/routing still ahead).

## How the runtime controller relates to moon

The [runtime controller (Takogami)](runtime-controller.md) (`takogami`) is the WfOS runtime controller,
not a replacement for moon. moon is the project-graph and task backend; the runtime controller
routes higher-level workflow commands to moon's task graph (a compat backend) and to
[native-toolchain (Panoply)](native-toolchain.md) tools, recording sessions and applying policy
along the way. Native manifests and the moon graph stay authoritative for builds; the runtime
controller adds discovery, routing, and rails on top.

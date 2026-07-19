# Apps — docs site and marketing site

WfOS ships two small web surfaces, both built with [Zola](https://www.getzola.org/) — a fast
static-site generator in a single Rust binary with built-in Sass, syntax highlighting, and
search. One binary, markdown in, static site out: a clean fit for both the docs and a simple
landing page.

| App | Path | Purpose | Status |
|-----|------|---------|--------|
| Docs site | `apps/docs` | Render `docs/*` for humans to browse | planned |
| Marketing site | `apps/web` | One-page site to promote and share WfOS | planned |

## Why Zola

- Single binary, zero runtime dependencies — installs via proto, Homebrew, or `cargo`.
- Markdown-native, so the docs site sources the same `docs/*.md` already in the repo.
- Built-in Sass compilation, syntax highlighting, and search — no JS toolchain required.

## `apps/docs` — documentation site

Renders the workspace documentation as a browsable site.

```txt
apps/docs/
  config.toml        site config (base_url, title, search)
  content/           docs content — sourced from ../../docs/*.md
  templates/         minimal layout + nav
  static/            assets
```

The `content/` tree is populated from the repo `docs/` (a sync/symlink step copies the
markdown in at build time, so the docs stay single-source). Local preview and build:

```bash
zola serve     # live preview at http://127.0.0.1:1111
zola build     # static output in public/
```

## `apps/web` — marketing site

A single page to start: tagline, the five archetypes/products, and links to the docs and repository.
Same Zola commands; grows into more pages only if it earns them.

## Build integration

Once scaffolded, each app gets a `moon.yml` with `build` and `serve` tasks and is added to
the moon project graph (see [monorepo.md](monorepo.md)). Add `zola` (and `node`, if a theme
needs it) to `.prototools` at that point.

## Status and follow-up

This pass documents structure and commands only. The actual Zola scaffold — `config.toml`,
content sync, templates, and theme — is the follow-up implementation step. Until then,
`apps/docs` and `apps/web` are README stubs. Per [agent-rails.md](agent-rails.md), agents do
not run `zola serve`/`build` without explicit permission.

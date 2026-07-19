# Setup

WfOS is local-first and modular. You can adopt the whole workspace or just one package — keep
your own shell, prompt, and editor, and let WfOS slot in beside them.

## Prerequisites

Install **proto** and **moon** first; proto then installs the pinned toolchains.

```bash
# proto (toolchain manager) — see https://moonrepo.dev/proto
curl -fsSL https://moonrepo.dev/install/proto.sh | bash

# moon is pinned in .prototools and fetched by `proto install`
```

## First run

From the workspace root:

```bash
moon run wfos:setup     # proto install — fetch pinned toolchains (proto, moon, rust)
moon run panoply:doctor    # detect tools, print readiness, write the Ontarch registry
```

`panoply:doctor` is read-only and safe to run anytime. To install missing Panoply tools and wire
your shell (human-only):

```bash
packages/panoply/bin/panoply bootstrap          # or: --dry-run to preview
```

After `bootstrap`, `panoply` is on `PATH` (symlinked into `~/.local/bin`), so you can call
`panoply doctor` from anywhere.

## PANOPLY_HOME and Workstreams layout

Panoply resolves its package from the script location at runtime (`panoply doctor`, `bootstrap`, etc.).
The shell fragment [`config/shell/panoply.zsh`](../packages/panoply/config/shell/panoply.zsh) also defines a
**suggested** default when `PANOPLY_HOME` is unset:

```txt
~/Workstreams/Build/src/workspaces/wfos/packages/panoply
```

That path is a convention, not a requirement. If your wfos clone lives elsewhere, export
`PANOPLY_HOME` before the shell loads (typically in `~/.zshenv` or `~/.zprofile`):

```bash
export PANOPLY_HOME="$HOME/path/to/wfos/packages/panoply"
```

`panoply bootstrap` writes `export PANOPLY_HOME="<resolved path>"` into `~/.zshrc` from the actual
package location, so bootstrap users get the correct path automatically. Chezmoi-managed
`.zshrc` uses the same suggested default with the same override — see
[`packages/panoply/dotfiles/README.md`](../packages/panoply/dotfiles/README.md).

The `Workstreams/` tree layout (`Plan/`, `Brand/`, `Build/`, `Control/`) is documented in
[architecture.md](architecture.md#workstreams-collection) as a typical arrangement; mount points
and namespace paths are yours to choose.

## mise / proto coexistence

proto pins the workspace build toolchains (`.prototools`). [Panoply](native-toolchain.md) uses **mise** as
its runtime manager for day-to-day shells and activates it in `config/shell/panoply.zsh`. The two
coexist: activation order lets mise manage Panoply-scoped runtimes while proto handles the
workspace. Nothing is removed from your existing setup.

## Modular adoption

You do not have to take all of WfOS:

- Want just the tool substrate? Use [Panoply](native-toolchain.md) (`panoply doctor` / `bootstrap`) and ignore
  the rest.
- Want the metadata contracts? Use [Ontarch](metadata-plane.md) descriptors/policies in your own tooling.
- Want the monorepo conventions? Use the [moon + proto](monorepo.md) skeleton.

Adopt one piece, keep your own workflow, and grow into more when it earns its place.

## AI skills (planned setup flow)

The planned CLI setup flow lets you choose which AI enhancements to install, each with a
description (see the AI section of [tool-catalog.md](tool-catalog.md)). They are all opt-in —
RTK, ponytail, drawio-skill, SkillSpector, Handy, improve, OpenRouter, Fabric — so you install
only what fits your workflow. Until the flow ships, install any of them directly per their
upstream instructions. Scan third-party skills with SkillSpector before trusting them.

## Agent mode

Agents run with `PANOPLY_AGENT=1`, which allows read-only commands and blocks installs, secret
reads, and dotfile edits. See [agent-rails.md](agent-rails.md) for the full policy.

```bash
PANOPLY_AGENT=1 packages/panoply/bin/panoply doctor   # ok
PANOPLY_AGENT=1 packages/panoply/bin/panoply bootstrap # blocked
```

## Core workflow apps (separate, documented install)

The native writing/notes/AI stack — Logseq, Obsidian, Typst, aichat, Ollama, OpenRouter — is a
**documented recommendation, not part of `panoply bootstrap`**. Install it yourself when you want
it; the full guide and quick-start commands are in [workflow-apps.md](workflow-apps.md). The
markdown-on-disk source of truth keeps every choice swappable.

## Apps preview

Once the [Zola apps](apps.md) are scaffolded:

```bash
cd apps/docs && zola serve   # docs site preview
cd apps/web  && zola serve   # marketing site preview
```

## Troubleshooting

- `moon: command not found` — install proto, then `proto install`; ensure `~/.proto/shims` is
  on `PATH`.
- `panoply:doctor` shows missing defaults — run `panoply bootstrap` (or install the listed Homebrew
  formulae manually).
- moon does not see a new project — add it to `projects.sources` in `.moon/workspace.yml` and
  give it a `moon.yml` (see [monorepo.md](monorepo.md)).

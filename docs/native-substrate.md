# Native substrate — Dust

Dust is the layer of small native Unix/Rust tools that make the machine usable for both
developers and AI agents. It is shell-first and installs tools **globally** (via Homebrew +
mise); only the manifest, scripts, config templates, and metadata live in this repo.

Dust is where work is physically executed on the machine. [Kraken](runtime-controller.md) controls and
routes; Dust runs the native commands.

## What lives in the package

```txt
manifest/dust.tools.toml   single source of truth — modules + tools
bin/                       dispatcher: dust, dust-doctor, dust-bootstrap, dust-env
lib/                       manifest parser, shared helpers, per-module logic
config/                    Brewfile + shell fragment + tool config templates
moon.yml                   doctor/list/env tasks for the project graph
```

The generated tool registry is written to the [Archon](metadata-plane.md) package
(`packages/archon/registry/tools.json`) and is host-specific (gitignored).

## Commands

| Command | Mutating | Agent-safe | Purpose |
|---------|----------|------------|---------|
| `dust doctor` | no | yes | detect tools, print readiness, write the Archon registry |
| `dust list [module]` | no | yes | list modules and tools from the manifest |
| `dust env` | no | yes | print the shell activation snippet |
| `dust bootstrap [--dry-run]` | yes | no | install missing tools (brew + mise), symlink configs, wire `~/.zshrc` |

Run them directly or through moon: `moon run dust:doctor`, `moon run dust:list`,
`moon run dust:env`.

## Modules

Each module groups related tools and is replaceable. Defaults are what `bootstrap` installs;
alternatives are detected if present but never forced.

| Module | Default | Alternatives | Purpose |
|--------|---------|--------------|---------|
| `shell` | starship, shellcheck, zsh-autosuggestions, zsh-syntax-highlighting | zsh-autocomplete | prompt context, script linting, shell UX plugins |
| `git` | git, gh | jj, lazygit, git-delta | version control and source status |
| `nav` | fzf, zoxide, eza, bat, ripgrep, fd, jq, tldr | skim, choose | search, selection, navigation, cheatsheets |
| `system` | btop, dua | — | resource monitor and disk-usage visualizer |
| `session` | tmux | zellij | persistent terminal sessions |
| `secrets` | pass | age, sops | local secret handling (agent-blocked) |
| `tools` | mise, direnv | proto, asdf | runtime/version + per-dir env |
| `dotfiles` | — | chezmoi | cross-machine dotfile management |
| `js` | node, pnpm | bun, deno, aube | JS/TS runtime and package routing |
| `rust` | cargo | rustup, cargo-nextest | Rust build/test routing |
| `ether` | wasmtime | — | WASM/WASI runtime for portable components |
| `logs` | files | sqlite3 | session and command traceability |

The `dust` disk tool from the Unix-substrate research is intentionally **substituted by `dua`**
to avoid a CLI name clash with the Dust product binary (`~/.local/bin/dust`).

The manifest (`manifest/dust.tools.toml`) is the authoritative list, with per-tool `brew`,
`detect`, `agent_safe`, and `alternatives` fields. `doctor` reads it to produce the registry;
see the catalog in [tool-catalog.md](tool-catalog.md) for descriptions and links.

### Detection forms

`doctor` and `bootstrap` resolve the manifest `detect` field through `dust_detect()`, which
handles three forms so non-binary tools report honestly:

| `detect` value | Check | Used by |
|----------------|-------|---------|
| `name` (no slash) | `command -v name` on `PATH` | most CLIs |
| `/abs/path` | absolute file/dir exists | absolute installs |
| `rel/path` | exists under `${HOMEBREW_PREFIX}` | sourced zsh plugins (`share/…`) |

The relative form lets sourced plugins (zsh-autosuggestions, zsh-syntax-highlighting,
zsh-autocomplete) appear in the registry even though they are not on `PATH`. Path-detected tools
report `installed` without a version string (there is no binary to query).

## Install model

Dust installs tools **globally** and keeps only sources of truth in the repo:

- Homebrew formulae (`config/Brewfile`)
- runtimes via mise
- `~/.config` symlinks for tool configs (starship, tmux, mise)
- one sourced line in `~/.zshrc` pointing at `config/shell/dust.zsh`

This matches the principle that low-level tools and dotfiles live on the machine, not inside
a project tree. `bootstrap` is the only command that writes outside the repo, and it is
human-only.

## Shell activation

`config/shell/dust.zsh` is sourced from `~/.zshrc`. Every activation is guarded so the file
is safe to source even when a tool is missing — it puts the Dust CLI on `PATH` and wires
mise, direnv, starship, zoxide, fzf, and modern coreutils aliases (`eza`, `bat`) only when
each is installed.

**`DUST_HOME`** points at the Dust package root (manifest, `bin/`, `config/`). If unset,
`dust.zsh` falls back to a suggested path under `~/Workstreams/Build/src/workspaces/wfos/…`.
Override with `export DUST_HOME=…` in `~/.zshenv` when your clone lives elsewhere;
`dust bootstrap` exports the resolved path from the running package automatically.

## mise / proto coexistence

Dust standardizes on **mise** as its runtime manager and activates it in the Dust shell
fragment. If a machine already uses **proto**, Dust does not remove it: activation order lets
mise manage Dust-scoped runtimes while proto stays available for existing workflows. To
retire proto later, remove its block from `~/.zshrc` and its `PATH` entry. (Note: proto also
pins the workspace's own build toolchains — see [monorepo.md](monorepo.md).)

## chezmoi coexistence

Dust's own config flow stays the default: `bootstrap` symlinks a small set of `~/.config`
templates and wires one sourced line into `~/.zshrc`. [chezmoi](https://www.chezmoi.io/) is
offered as an **optional** tool in the `dotfiles` module for operators who want full
cross-machine dotfile management (templating, per-host variants, encrypted secrets) on top of
or instead of the symlink flow. It is never auto-installed and Dust does not require it.

A draft chezmoi source tree lives at [`packages/dust/dotfiles/`](../packages/dust/dotfiles/README.md).
It defines four profile classes (`$WFOS_PROFILE`, default `local-macos-full`) that renders a
narrower environment for `headless-dev` and `agent-safe` machines. Validate without touching
`$HOME`:

```bash
bash packages/dust/dotfiles/bin/validate.sh
bash packages/dust/dotfiles/bin/validate-dotfiles.sh      # + profile preview when chezmoi is installed
moon run dust:validate-dotfiles
```

Promotion to `~/.local/share/chezmoi/` and `chezmoi apply` are human-gated — see the dotfiles
README for the promotion workflow.

## Agent rails

`dust` reads the `DUST_AGENT` environment variable. In agent mode, read-only commands
(`doctor`, `list`, `env`, `version`, `help`) run; mutating ones (`bootstrap`, installs,
secret reads) are blocked. The rules live in the Archon policy
`packages/archon/policies/dust.agent.policy.toml`; see [agent-rails.md](agent-rails.md).

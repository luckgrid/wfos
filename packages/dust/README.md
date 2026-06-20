# Dust — native substrate

Dust is the layer of small native Unix/Rust tools that make the machine usable for developers
and AI agents. It is shell-first and installs tools **globally** (Homebrew + mise); only the
manifest, scripts, config templates, and metadata live here.

Deep dive: [`../../docs/native-substrate.md`](../../docs/native-substrate.md).

## Layout

```txt
manifest/dust.tools.toml   single source of truth — modules + tools
bin/                       dust, dust-doctor, dust-bootstrap, dust-env
lib/                       manifest parser, shared helpers, per-module logic
config/                    Brewfile + shell fragment + tool config templates
moon.yml                   doctor/list/env tasks
```

The generated tool registry is written to the [Archon](../archon/README.md) package
(`packages/archon/registry/tools.json`) — host-specific and gitignored.

## Quick start

```bash
moon run dust:doctor          # detect + report (read-only); writes the Archon registry
moon run dust:list            # list modules and tools
bin/dust bootstrap            # install missing tools + wire shell (human-only; --dry-run to preview)
```

After `bootstrap`, `dust` is on `PATH` (symlinked into `~/.local/bin`).

## Commands

| Command | Mutating | Agent-safe | Purpose |
|---------|----------|------------|---------|
| `dust doctor` | no | yes | detect tools, print readiness, write the registry |
| `dust list [module]` | no | yes | list modules and tools from the manifest |
| `dust env` | no | yes | print the shell activation snippet |
| `dust bootstrap` | yes | no | install (brew + mise), symlink configs, wire `~/.zshrc` |

## Modules

`shell, git, nav, session, secrets, tools, js, rust, ether, logs` — each replaceable
(fzf ↔ skim, tmux ↔ zellij, mise ↔ proto, git ↔ jj). The manifest holds per-tool `brew`,
`detect`, `agent_safe`, and `alternatives`. Descriptions and links:
[`../../docs/tool-catalog.md`](../../docs/tool-catalog.md).

## mise / proto coexistence

Dust standardizes on **mise** for day-to-day runtimes and activates it in
`config/shell/dust.zsh`; an existing **proto** setup is left intact. (proto also pins the
workspace build toolchains — see [`../../docs/monorepo.md`](../../docs/monorepo.md).)

## Agent rails

`dust` reads `DUST_AGENT`. In agent mode, read-only commands run; mutating ones are blocked
per `../archon/policies/dust.agent.policy.toml`. See [`AGENTS.md`](AGENTS.md) and
[`../../docs/agent-rails.md`](../../docs/agent-rails.md).

## Related

- [`../archon/README.md`](../archon/README.md) — metadata this package produces and is governed by
- [`../../docs/native-substrate.md`](../../docs/native-substrate.md) · [`../../docs/setup.md`](../../docs/setup.md)

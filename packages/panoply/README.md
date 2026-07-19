# Panoply — native toolchain

Panoply is the layer of small native Unix/Rust tools that make the machine usable for developers
and AI agents. It is shell-first and installs tools **globally** (Homebrew + mise); only the
manifest, scripts, config templates, and metadata live here.

Deep dive: [`../../docs/native-toolchain.md`](../../docs/native-toolchain.md).

## Layout

```txt
manifest/panoply.tools.toml   single source of truth — modules + tools
bin/                       panoply, panoply-doctor, panoply-bootstrap, panoply-env, panoply-gen, validate-substrate.sh
lib/                       manifest parser, generate.sh, shared helpers, per-module logic
config/                    Brewfile (generated) + shell fragment + tool config templates
moon.yml                   doctor/list/env/gen-check/validate-substrate tasks
```

The generated tool registry is written to the [Ontarch](../ontarch/README.md) package
(`packages/ontarch/registry/tools.json`) — host-specific and gitignored.

## Quick start

```bash
moon run panoply:doctor          # detect + report (read-only); writes the Ontarch registry
moon run panoply:list            # list modules and tools
bin/panoply bootstrap            # install missing tools + wire shell (human-only; --dry-run to preview)
```

After `bootstrap`, `panoply` is on `PATH` (symlinked into `~/.local/bin`).

## PANOPLY_HOME

The shell fragment sets a suggested default (`~/Workstreams/Build/src/workspaces/wfos/packages/panoply`)
when `PANOPLY_HOME` is unset. Override in `~/.zshenv` if your layout differs; `bootstrap` exports
the resolved package path into `~/.zshrc` automatically. See [`../../docs/setup.md`](../../docs/setup.md#panoply_home-and-workstreams-layout).

## Commands

| Command | Mutating | Agent-safe | Purpose |
|---------|----------|------------|---------|
| `panoply doctor [--json] [--no-write]` | no | yes | detect tools, print readiness (`--json` = parseable), write the registry |
| `panoply list [module]` | no | yes | list modules and tools from the manifest |
| `panoply gen <brewfile\|mise>` | no | yes | derive install artifacts from the manifest (dry-run, stdout) |
| `panoply env [--shell\|--json]` | no | yes | print the resolved environment (paths, module map, `PANOPLY_AGENT`); `--shell` = activation snippet |
| `panoply bootstrap` | yes | no | install (brew + mise), symlink configs, wire `~/.zshrc` |

The manifest is the single source of truth: `panoply gen brewfile` reproduces `config/Brewfile`
exactly (enforced by `panoply gen brewfile --check` / `moon run panoply:gen-check`).

## Modules

`shell, git, nav, system, session, secrets, tools, dotfiles, js, rust, wisp, logs, agent` —
each replaceable (fzf ↔ skim, tmux ↔ zellij, mise ↔ proto, git ↔ jj). The manifest holds
per-tool `brew`, `detect`, `agent_safe`, and `alternatives`. Descriptions and links:
[`../../docs/tool-catalog.md`](../../docs/tool-catalog.md).

The `agent` module wires **RTK** as the recommended-default output compressor (60-90% token
savings), swappable via `PANOPLY_RTK` / profile data — see
[`../../docs/native-toolchain.md`](../../docs/native-toolchain.md#output-compression-rtk).

## mise / proto coexistence

Panoply standardizes on **mise** for day-to-day runtimes and activates it in
`config/shell/panoply.zsh`; an existing **proto** setup is left intact. (proto also pins the
workspace build toolchains — see [`../../docs/monorepo.md`](../../docs/monorepo.md).)

## Agent rails

`panoply` reads `PANOPLY_AGENT`. In agent mode, read-only commands run; mutating ones are blocked
per `../ontarch/policies/panoply.agent.policy.toml`. See [`AGENTS.md`](AGENTS.md) and
[`../../docs/agent-rails.md`](../../docs/agent-rails.md).

## Related

- [`dotfiles/README.md`](dotfiles/README.md) — chezmoi source (profiles, validation, promotion)
- [`dotfiles/SECRETS.md`](dotfiles/SECRETS.md) — tiered vault model + agent secret-read hard block
- [`secrets/README.md`](secrets/README.md) — sops + age fixtures (files vault)
- [`../ontarch/README.md`](../ontarch/README.md) — metadata this package produces and is governed by
- [`../../docs/native-toolchain.md`](../../docs/native-toolchain.md) · [`../../docs/setup.md`](../../docs/setup.md)

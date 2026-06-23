# WfOS agent guide

Keep this file lean and directive. [`README.md`](README.md) and [`docs/`](docs/README.md) are
the source of truth for detailed commands and architecture.

## Core rules

- **Local-first moonrepo.** Toolchains are pinned in [`.prototools`](.prototools) and installed
  by proto. Install **proto** and **moon** first; on a fresh clone run `moon run wfos:setup`.
- **Run from the workspace root** unless a package/app README says otherwise.
- **Native manifests stay authoritative.** Archon describes meaning, routing, and policy; it
  never replaces `Cargo.toml`, `package.json`, `mise.toml`, or lockfiles.
- **Stay within the rails.** Agents run with `DUST_AGENT=1`: read-only commands are allowed;
  installs, secret reads, and dotfile edits are blocked. See [docs/agent-rails.md](docs/agent-rails.md).

## What agents may / may not do

| Allowed (read-only) | Blocked (human-only) |
|---------------------|----------------------|
| `dust doctor`, `dust list`, `dust env` | `dust bootstrap`, brew/mise installs |
| `moon run dust:doctor`, `moon query …` | reading secrets (`pass`/`age`/`sops`) |
| read descriptors, schemas, policies, registry | editing `~/.zshrc` or `~/.config` symlinks |
| read/edit files in this repo | starting servers / `zola serve` / long-running dev tasks |

Gates and the policy that enforces them live at
`packages/archon/policies/dust.agent.policy.toml`.

## Key paths

- Toolchain pins: [`.prototools`](.prototools)
- Project graph + tasks: [`.moon/`](.moon/), root [`moon.yml`](moon.yml), per-project `moon.yml`
- Native substrate: [`packages/dust/`](packages/dust/AGENTS.md) — manifest, scripts, configs
- Metadata plane: [`packages/archon/`](packages/archon/AGENTS.md) — descriptors, schemas, policies, registry
- Documentation: [`docs/`](docs/README.md)

## Workspaces

- **`packages/*`** — shared infrastructure; keep interfaces stable and composable. Validate
  with the project's moon tasks before relying on dependents.
- **`apps/*`** — each app owns its ports, env, and build/serve commands; do not run them
  without explicit permission.

## Skills

Agent skills are third-party code. Scan with
[SkillSpector](https://github.com/nvidia/skillspector) before trusting a skill, the same way
you would review a dependency. Optional AI enhancements are catalogued in
[docs/tool-catalog.md](docs/tool-catalog.md).

## Learned User Preferences

- WfOS public docs and READMEs must be self-contained: do not link to Build/bin or Plan/bin spec
  files; cite in-repo paths, published URLs, or conceptual namespace names only (session JSON
  provenance may keep bin refs).
- In user-facing wfos docs, replace epic IDs (E01, E02, etc.) with wfos-native terms (secrets
  module, dust bootstrap, archon, etc.).

## Learned Workspace Facts

- Chezmoi profile exclusions live in `.chezmoiignore.tmpl` (not bare `.chezmoiignore`); use a
  dict+range template pattern so YAML linters do not mis-parse the file.
- Draft chezmoi source: `packages/dust/dotfiles/`; promotion to `$HOME` /
  `~/.local/share/chezmoi/` is human-gated (agent rails block apply).
- `DUST_HOME` default suggests `~/Workstreams/Build/src/workspaces/wfos/packages/dust`;
  override in `~/.zshenv` when your layout differs; `bootstrap` exports the resolved path.
- Local dotfiles testing: `packages/dust/dotfiles/bin/validate-dotfiles.sh` (optional `--apply` for temp
  HOME smoke test); `moon run dust:validate-dotfiles`.

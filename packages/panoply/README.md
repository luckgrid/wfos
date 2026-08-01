# `native-toolchain` — Panoply 🧰

Panoply is WfOS's native-toolchain package: small Unix and Rust tools that make a developer
machine usable without replacing the shell, operating system, or package managers. It installs
host tools globally through Homebrew and mise; this package owns the manifest, scripts,
configuration templates, and validation logic.

This README is the entrypoint for every worker touching the package. Deep reference:
[`../../docs/native-toolchain.md`](../../docs/native-toolchain.md).

## Authority and safety

- `manifest/panoply.tools.toml` is the single source of truth for modules and tools.
- Generated Brewfile, registry, and shell artifacts are derived from the manifest; regenerate
  them instead of hand-editing them.
- Run package tasks from the WfOS workspace root unless a command below uses a package-local path.
- `panoply bootstrap` changes the host and is human-gated.
- Automated workers must obey the selected profile and
  [`../ontarch/policies/panoply.agent.policy.toml`](../ontarch/policies/panoply.agent.policy.toml).

## Layout

```text
manifest/panoply.tools.toml   authoritative modules and tools
bin/                           CLI, doctor, bootstrap, env, generation, validation
lib/                           parser, generation, helpers, and module logic
config/                        generated Brewfile, shell fragment, and templates
dotfiles/                      chezmoi source and routing contracts
secrets/                       sops and age fixtures
moon.yml                       package tasks
```

The generated tool registry is written under the
[metadata-plane package](../ontarch/README.md) at `packages/ontarch/registry/tools.json`. It is
host-specific and gitignored.

## First commands

```bash
moon run panoply:doctor              # detect and report; writes the generated tool registry
moon run panoply:list                # list modules and tools
moon run panoply:gen-check           # prove generated install artifacts match the manifest
moon run panoply:validate-substrate  # package validation gate

packages/panoply/bin/panoply bootstrap --dry-run  # human preview
```

After a human runs `bootstrap`, `panoply` is symlinked into `~/.local/bin`.

## Commands and rails

| Command | Mutating | Automated-worker default | Purpose |
|---|---:|---|---|
| `panoply doctor [--json] [--no-write]` | no | allowed | detect tools and report readiness |
| `panoply list [module]` | no | allowed | list manifest modules and tools |
| `panoply gen <brewfile\|mise>` | no | allowed | derive install artifacts to stdout |
| `panoply env [--shell\|--json]` | no | allowed | print resolved environment data |
| `panoply bootstrap` | yes | blocked | install tools, link config, and modify shell setup |

With `PANOPLY_AGENT=1`, Panoply permits the read-only commands and blocks bootstrap, installs,
secret reads, and shell or dotfile mutation. The policy—not this reminder table—is authoritative.
See [`../../docs/agent-rails.md`](../../docs/agent-rails.md).

## Editing rules

- Add or change tools in `manifest/panoply.tools.toml`.
- Keep scripts POSIX/bash compatible and `shellcheck` clean.
- Preserve guarded shell activation so configuration remains safe when an optional tool is absent.
- Never read `pass`, `age`, or `sops` secrets from an automated session unless the selected
  profile and policy explicitly elevate that boundary.
- Never edit `~/.zshrc`, `~/.config`, or host symlinks as a side effect of a read-only task.

## `PANOPLY_HOME`

The shell fragment suggests
`~/Workstreams/Build/src/workspaces/wfos/packages/panoply` when `PANOPLY_HOME` is unset. This is a
convention, not a mandatory layout. Override it in `~/.zshenv` when needed. Details:
[`../../docs/setup.md`](../../docs/setup.md#panoply_home-and-workstreams-layout).

## Modules

```text
shell · git · nav · system · session · secrets · tools · dotfiles · js · rust · wisp · logs · agent
```

Implementations are replaceable. The manifest records package source, detection, automated-worker
safety, and alternatives. Tool descriptions and links live in
[`../../docs/tool-catalog.md`](../../docs/tool-catalog.md).

## Related

- [`dotfiles/README.md`](dotfiles/README.md) — chezmoi source, profiles, validation, and promotion
- [`dotfiles/SECRETS.md`](dotfiles/SECRETS.md) — vault model and secret-read boundary
- [`secrets/README.md`](secrets/README.md) — sops and age fixtures
- [`../ontarch/README.md`](../ontarch/README.md) — metadata and policy authority
- [`../../docs/worker-guidance.md`](../../docs/worker-guidance.md) — repository-wide conventions

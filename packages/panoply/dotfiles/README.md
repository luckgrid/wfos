# Panoply dotfiles — chezmoi source

This directory is the **chezmoi source state** for the WfOS machine-config substrate managed
by [Panoply](../README.md). It is a **draft** — not yet promoted to a live install. Nothing here
edits `~/.zshrc`, `~/.config`, or any other live dotfile until a human runs promotion (below).

> **Draft posture:** promotion to the live source dir `~/.local/share/chezmoi/` is a
> **human-gated** step (see "Promotion" below). Agents must not apply this to `$HOME`; that is
> blocked by the Panoply agent rails
> ([`packages/ontarch/policies/panoply.agent.policy.toml`](../../ontarch/policies/panoply.agent.policy.toml)
> → `no_shell_mutation`).

## Why chezmoi (vs a bare `$HOME` git repo)

[chezmoi](https://www.chezmoi.io/) (MIT) works out of an **isolated source dir** and *renders*
targets on demand, instead of overlaying a bare git repo on `$HOME` (yadm's model). That isolation
lets one source tree render a different environment per [profile class](#profile-classes)
(`local-macos-full`, `headless-dev`, `agent-safe`, `workstreams-maintainer`), so an agent reads
**one profile declaration** instead of crawling `~/.zshrc` + `~/.gitconfig` + a sprawl of
`~/.config/*`.

See also [chezmoi coexistence](../../../docs/native-toolchain.md#chezmoi-coexistence) in the
native-toolchain docs.

## Source layout

```txt
dotfiles/                         # chezmoi source root (this dir)
├── .chezmoi.toml.tmpl            # generates ~/.config/chezmoi/chezmoi.toml; declares `profile` data
├── .chezmoidata/profiles.toml    # the four profile classes + their render/exclude declarations
├── .chezmoidata/routing.toml     # machine-readable config routing contract (apps consume profiles)
├── .chezmoidata/vaults.toml      # machine-readable tiered vault contract (pass vs sops+age)
├── .chezmoiignore.tmpl           # templated: excludes targets per profile (the exclusion mechanism)
├── ROUTING.md                    # config routing rules: shared intent vs app syntax, no secrets
├── SECRETS.md                    # tiered vault model + agent secret-read hard block
├── dot_zshrc.tmpl                # -> ~/.zshrc   (sources the Panoply shell fragment, guarded)
├── dot_gitconfig.tmpl            # -> ~/.gitconfig
├── dot_config/zsh/plugins.zsh.tmpl  # guarded, profile-aware zsh plugin stack
├── dot_config/                   # starship, mise, tmux, zed, opencode (added after first clean dry run)
└── private_dot_config/wfos-secrets/ # sops/pass secret *references* only — never values (resolved on apply)
```

The file-oriented `sops` + `age` vault fixtures (`.sops.yaml` + structure-only sample) live in
[`../secrets/`](../secrets/README.md); see [`SECRETS.md`](SECRETS.md) for the tiered model.

## Profile classes

One source tree renders a different environment per profile. The profile is chosen by the
`$WFOS_PROFILE` env var (default `local-macos-full`), declared as the `profile` data var in
`.chezmoi.toml.tmpl`, and resolved against `.chezmoidata/profiles.toml` (chezmoi's native
auto-loaded `.chezmoidata/` location).

| Profile | Use case | Renders | Excludes |
|---------|----------|---------|----------|
| `local-macos-full` | main workstation | everything | — |
| `headless-dev` | remote server / VM | shell, git, prompt, session, runtime | editor-gui, agent-tools, secrets |
| `agent-safe` | agent execution (`PANOPLY_AGENT=1`) | shell, git (non-secret) | prompt, session, runtime, editor-gui, agent-tools, secrets |
| `workstreams-maintainer` | toolkit maintenance | everything + docs/tools | — |

Exclusions are **enforced by construction**: `.chezmoiignore.tmpl` reads the current
profile's `excludes` list and tells chezmoi not to render those targets, so an `agent-safe` or
`headless-dev` machine never materializes GUI or secret configs — that context cannot enter an
agent's prompt. `agent-safe` additionally declares `secrets = false`, `remote_writes = false`,
`gui = false`, and its `dot_gitconfig` renders `push.default = nothing`.

## zsh plugin stack

Bare zsh + standalone plugins (no Oh My Zsh, no Powerlevel10k; prompt stays Starship). The chezmoi
layer owns the plugin integration via `dot_config/zsh/plugins.zsh.tmpl`, sourced from `dot_zshrc`:

- `zsh-autosuggestions` (MIT) and `zsh-syntax-highlighting` (BSD-3) — sourced guarded; highlighting
  is always **last** so it wraps the final widget set.
- `zsh-autocomplete` (MIT) — optional and conflict-prone, so opt-in (`WFOS_ZSH_AUTOCOMPLETE=1`) and
  only on full/maintainer profiles, never `headless-dev` or `agent-safe`.
- `agent-safe` loads **no** interactive plugins (minimal, non-interactive surface).
- Every `source` is guarded (`[ -f ... ] && source`): a missing plugin never breaks shell startup.

`dot_zshrc` sets `PANOPLY_PLUGINS_MANAGED=1` before sourcing `panoply.zsh`, so the substrate fragment
stands down and this profile-aware fragment is the single authority (no double-sourcing). Plugin
*install* is owned by [`panoply bootstrap`](../README.md) and the manifest `dotfiles` module; this
directory owns only the shell integration.

Chezmoi naming conventions used here:

| Source name | Renders to | Meaning |
|-------------|-----------|---------|
| `dot_zshrc` | `~/.zshrc` | `dot_` → leading `.` |
| `*.tmpl` | rendered | Go-template processed at apply time |
| `private_*` | mode `0600` | private; never world/group readable |
| `.chezmoi*` | (not a target) | chezmoi control files (config/data) |

## Validate (dry-run gate, no `$HOME` writes)

```bash
bin/validate.sh              # 7-check gate only (structure, profiles, routing, …)
bin/validate-dotfiles.sh     # gate + per-profile preview (chezmoi required for preview)
bin/validate-dotfiles.sh --apply   # also smoke-test chezmoi apply in a temp HOME
```

From the workspace root: `moon run panoply:validate-dotfiles`

`validate.sh` never writes to `$HOME`. When the `chezmoi` binary is present it additionally runs
`chezmoi execute-template` against a throwaway source copy to prove templates render; when it is
absent (install via `panoply bootstrap` / manifest `dotfiles` module) it records that the live
`chezmoi diff` is deferred.

## Promotion (human-gated)

Set `PANOPLY_HOME` if wfos is not under the suggested `~/Workstreams/Build/…` layout (or rely on
the default in `dot_zshrc.tmpl`). `panoply bootstrap` exports the resolved path for you.

```bash
# Human, on a host with chezmoi installed (panoply bootstrap / dotfiles module):
export PANOPLY_HOME="${PANOPLY_HOME:-$HOME/Workstreams/Build/src/workspaces/wfos/packages/panoply}"
chezmoi init --source <this dir> --dry-run     # preview only
chezmoi diff --source <this dir>               # review the would-be changes
chezmoi apply --source <this dir>              # WRITE to $HOME — explicit human action only
```

## Related

- [`SECRETS.md`](SECRETS.md) — tiered vault model + agent secret-read hard block
- [`ROUTING.md`](ROUTING.md) — config routing rules (shared intent vs app syntax)
- [`../config/shell/panoply.zsh`](../config/shell/panoply.zsh) — the Panoply activation fragment `dot_zshrc` sources
- [`../../../docs/native-toolchain.md`](../../../docs/native-toolchain.md) — native toolchain + chezmoi coexistence
- [`../../ontarch/README.md`](../../ontarch/README.md) — metadata plane (descriptors, policies, registry)

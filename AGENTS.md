# WfOS agent guide

Keep this file lean and directive. [`README.md`](README.md) and [`docs/`](docs/README.md) are
the source of truth for detailed commands and architecture.

## Core rules

- **Local-first moonrepo.** Toolchains are pinned in [`.prototools`](.prototools) and installed
  by proto. Install **proto** and **moon** first; on a fresh clone run `moon run wfos:setup`.
- **Run from the workspace root** unless a package/app README says otherwise.
- **Native manifests stay authoritative.** Ontarch describes meaning, routing, and policy; it
  never replaces `Cargo.toml`, `package.json`, `mise.toml`, or lockfiles.
- **Stay within the rails.** Agents run under a profile (`Workstreams/.agents/profiles/`); the
  profile's `rails` selects the Ontarch policy that bounds scope, commands, and secrets. The
  default `workspace-dev` profile runs with `PANOPLY_AGENT=1`: read-only commands are allowed;
  installs, secret reads, and dotfile edits are blocked. See
  [docs/agent-configs.md](docs/agent-configs.md) and [docs/agent-rails.md](docs/agent-rails.md).

## What agents may / may not do

| Allowed (read-only) | Blocked (human-only) |
|---------------------|----------------------|
| `panoply doctor`, `panoply list`, `panoply gen`, `panoply env` | `panoply bootstrap`, brew/mise installs |
| `moon run panoply:doctor`, `moon run panoply:gen-check`, `moon run panoply:validate-substrate`, `moon query …` | reading secrets (`pass`/`age`/`sops`) |
| `moon run ontarch:validate`, `moon run ontarch:sync`, `moon run ontarch:scan`, `moon run ontarch:secrets-scan` | editing `~/.zshrc` or `~/.config` symlinks |
| read descriptors, schemas, policies, registry | starting servers / `zola serve` / long-running dev tasks |
| read/edit files in this repo | (other mutations require a human) |

Gates and the policy that enforces them live at
`packages/ontarch/policies/panoply.agent.policy.toml`.

## Key paths

- Toolchain pins: [`.prototools`](.prototools)
- Project graph + tasks: [`.moon/`](.moon/), root [`moon.yml`](moon.yml), per-project `moon.yml`
- Native toolchain: [`packages/panoply/`](packages/panoply/AGENTS.md) — manifest, scripts, configs
- Metadata plane: [`packages/ontarch/`](packages/ontarch/AGENTS.md) — descriptors, schemas, policies, registry
- Documentation: [`docs/`](docs/README.md)

## Workspaces

- **`packages/*`** — shared infrastructure; keep interfaces stable and composable. Validate
  with the project's moon tasks before relying on dependents.
- **`apps/*`** — each app owns its ports, env, and build/serve commands; do not run them
  without explicit permission.

## Profile

This workspace's default agent profile is **`workspace-dev`**
([`Workstreams/.agents/profiles/workspace-dev.toml`](../../../../.agents/profiles/workspace-dev.toml)):
edit repo code, run `moon` tasks, stage commits locally (no push), no secret reads. The profile —
not this file — is the source of truth for scope and command rules; see
[docs/agent-configs.md](docs/agent-configs.md).

## Skills

Agent skills are third-party code. Scan with
[SkillSpector](https://github.com/nvidia/skillspector) before trusting a skill, the same way
you would review a dependency — an unscanned skill does not load, and skill-loading profiles
carry `skillspector_scan` in `required_validators`. Optional AI enhancements are catalogued in
[docs/tool-catalog.md](docs/tool-catalog.md).

## Learned User Preferences

- WfOS public docs and READMEs must be self-contained: do not link to Build/bin or Plan/bin spec
  files; cite in-repo paths, published URLs, or conceptual namespace names only (session JSON
  provenance may keep bin refs).
- In user-facing wfos docs, replace epic IDs (E01, E02, etc.) with wfos-native terms (secrets
  module, panoply bootstrap, ontarch, etc.).
- Suggested Workstreams layout paths in docs, descriptors, and shell defaults are conventions
  only; document override points (`PANOPLY_HOME`, mount points), never imply one canonical filesystem
  layout.
- When verifying epic or story completion, compare the repo to Build/bin specs and
  `packages/ontarch/registry/sessions`; consult Plan/bin only for extra context.

## Learned Workspace Facts

- Chezmoi profile exclusions live in `.chezmoiignore.tmpl` (not bare `.chezmoiignore`); use a
  dict+range template pattern so YAML linters do not mis-parse the file.
- Draft chezmoi source: `packages/panoply/dotfiles/`; promotion to `$HOME` /
  `~/.local/share/chezmoi/` is human-gated (agent rails block apply).
- `PANOPLY_HOME` default suggests `~/Workstreams/Build/src/workspaces/wfos/packages/panoply`;
  override in `~/.zshenv` when your layout differs; `bootstrap` exports the resolved path.
- Local dotfiles testing: `packages/panoply/dotfiles/bin/validate-dotfiles.sh` (optional `--apply` for temp
  HOME smoke test); `moon run panoply:validate-dotfiles`.
- Secrets rails validation: `packages/panoply/dotfiles/bin/validate-secrets.sh`;
  `moon run panoply:validate-secrets`.
- Substrate gate: `packages/panoply/bin/validate-substrate.sh` (manifest derivation, doctor JSON,
  env, RTK, replaceability matrix); `moon run panoply:gen-check`, `moon run panoply:validate-substrate`.
- Ontarch generated registry (`packages/ontarch/registry/*.json`, `graph.dot`, `BIN-INVENTORY.md`)
  is gitignored; session records and `registry/QUERIES.md` stay tracked.
- Session records are `packages/ontarch/registry/sessions/YYYY-MM-DD-eNN-sN.json`; filename dates
  follow the local implementation/completion date established by nested-repo history, not a
  planned session date, document creation date, or the next UTC calendar day.
- `Workstreams/.agents/` is the operator navigation layer; Ontarch sync writes gitignored
  `tools/local-toolkit.yml`; Ontarch remains the routing authority.
- `no-agent-git-push` is Ontarch policy metadata (publish intent); `agent-git` is the cross-cutting
  git allow/gate/block policy (`applies_to = "agent"`). Profiles keep `panoply.agent` /
  `no-agent-git-push` as `rails` and must not contradict `agent-git` in `[commands]`. Runtime
  blocking of `git push`/`reset --hard`/`clean`/`gh` is deferred to Cthulhu, same boundary as
  direct secret CLI on `PATH`. See [docs/agent-rails.md](docs/agent-rails.md).
- Scoped profiles declare `[isolation]` (`worktree`/`branch`, `jj = "opt-in"`); isolation is
  declarative intent today — agents are not forcibly moved off the main worktree by Ontarch.
- Bin archive lifecycle: `moon run ontarch:bin-report` / `ontarch:bin-cleanup`; profiles select
  `agent-bin` via `rails_bin`; `archive`/`delete-approved` refuse under `PANOPLY_AGENT=1` — real FS
  mutation deferred to Cthulhu. See [docs/bin-archive.md](docs/bin-archive.md).

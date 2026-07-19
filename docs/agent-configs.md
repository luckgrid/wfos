# Agent configs & profiles

Agent apps (Claude, Cursor, Zed, Factory, OpenCode, CLI agents) each have their own prose config.
Left alone, the same intent — what an agent may touch, which commands it may run, whether it can
read secrets — gets restated five times. WfOS consolidates that intent into **shared profiles**
and keeps each `AGENTS.md` lean.

## Shared profiles

A profile is one declaration consumed by every app. Profiles live at
[`Workstreams/.agents/profiles/`](../../../../../.agents/profiles/README.md) as tracked TOML; each
declares scope (allowed/blocked paths), command allow/gate/block lists, secret access, a
remote-write policy, an `[isolation]` field (worktree/branch scope + jj opt-in), required
validators, an output compressor, and a session-log target. Apps consume the shared intent through
their own (chezmoi-rendered) config syntax — they never become a second policy source of truth.

[Ontarch](metadata-plane.md) policies remain the enforcement authority. A profile *selects* a
policy through its `rails` field and *scopes* it. The cross-cutting
[`agent-git`](../packages/ontarch/policies/agent-git.policy.toml) policy (`applies_to = "agent"`)
governs git allow/gate/block via the graph; profiles keep `panoply.agent` / `no-agent-git-push` as
`rails` and must not contradict `agent-git` in `[commands]`. Scoped agents declare
`[isolation] mode` as `worktree` or `branch` (not `main`) with `jj = "opt-in"` — see
[agent-rails.md](agent-rails.md#worktree-isolation). `ontarch validate` checks every profile against
`schemas/profile.schema.json`; `ontarch sync` flattens them into `registry/profiles.json` and draws
`profile → selects → policy` edges in the project graph.

## App integration pattern

```mermaid
flowchart TD
  Pol[Ontarch policies] -->|selected via rails| Reg[.agents/profiles]
  Reg --> Cursor[Cursor]
  Reg --> Zed[Zed]
  Reg --> Factory[Factory]
  Reg --> Claude[Claude]
  Reg --> OpenCode[OpenCode]
  Reg --> Shell[CLI agents]
  AppSyntax[app-specific syntax stays in app config] -. consumes .-> Reg
```

Rules:

- Keep shared policy in the registry.
- Keep app-specific syntax in app config.
- Do not duplicate secrets across agent configs.
- Do not let app configs bypass toolkit rails.
- Prefer one task per agent session.
- Require logs for autonomous routines.

The wiring from each app to the profile data is recorded in the routing contract
([`packages/panoply/dotfiles/.chezmoidata/routing.toml`](../packages/panoply/dotfiles/.chezmoidata/routing.toml)):
for every app, `consumes_profile_data = true` and `holds_secrets = false`.

Chezmoi app templates that exist today: Claude Code (`dot_claude/settings.json.tmpl`) and Zed
(`dot_config/zed/settings.json.tmpl`). Others are declared in the routing contract and land as
templates are added.

## Two profile layers (machine vs agent)

WfOS carries **two** profile concepts. They answer different questions and must not be conflated:

| Layer | Home | Question it answers | Consumed by |
|-------|------|-------------------|-------------|
| **Agent operating profile** | `Workstreams/.agents/profiles/*.toml` | What may this agent session touch? (scope, commands, rails, validators, compressor intent) | Agents, Ontarch (`ontarch validate` / `ontarch sync` → `profiles.json`), app renderers that read registry data |
| **Machine / chezmoi profile** | `packages/panoply/dotfiles/.chezmoidata/profiles.toml` | What config targets render on this host? (GUI, secrets, `rtk` shell hook) | chezmoi at render time (`local-macos-full`, `agent-safe`, …) |

```mermaid
flowchart TD
  AgentProf["Agent profile\n.agents/profiles/*.toml"]
  MachineProf["Machine profile\n.chezmoidata/profiles.toml"]
  Registry["registry/profiles.json"]
  Chezmoi["chezmoi templates\nper-app syntax"]
  AgentProf --> Registry
  AgentProf -. compressor intent .-> Registry
  MachineProf --> Chezmoi
  Registry -. read by apps/agents .-> Chezmoi
```

**RTK wiring today:** the agent profile `[output] compressor` field records compressor intent in
the registry (`output_compressor` in `profiles.json`). The Claude Code hook and the shell RTK
layer (`config/shell/rtk.zsh`) gate on the **machine** profile `rtk` flag in
`.chezmoidata/profiles.toml` (from the native-toolchain module). Until chezmoi bridges registry
data at render time, keep them aligned manually: set machine `rtk = true` when the active agent
profile declares `compressor = "rtk"`; set machine `rtk = false` to opt out on disk regardless of
registry intent. Skill-loading dev profiles (`workspace-dev`, `agent-safe-maintenance`) declare
`compressor = "rtk"`; `docs-only` omits it.

See [`packages/panoply/dotfiles/ROUTING.md`](../packages/panoply/dotfiles/ROUTING.md) for how app
templates consume machine profile data without becoming a policy source of truth.

## The lean `AGENTS.md` pattern

`AGENTS.md` is a **pointer, not a manual**. It carries only:

- core rules (substrate, run-from-root, native manifests stay authoritative, stay within rails),
- a short may / may-not table,
- key paths,
- the profile the workspace runs under,
- a skills note.

Detailed commands and architecture live in `README.md` and `docs/`, loaded on demand — so opening
`AGENTS.md` stays cheap. No app-specific prose duplicates profile intent: scope, command
allow/block lists, and secret rules are declared once in the profile, not retold per app or per
`AGENTS.md`. The copy-ready template is
[`.agents/profiles/AGENTS.template.md`](../../../../../.agents/profiles/AGENTS.template.md); the
reference instance is this workspace's [`AGENTS.md`](../AGENTS.md).

## Why it matters for token cost

Every app's prose config is context an agent loads. One profile means the same intent is declared
once and consumed everywhere, instead of duplicated as prose in five places. Scoped profiles also
load only the allowed paths and commands for a task, so irrelevant context never enters the
prompt, and a lean `AGENTS.md` keeps per-workspace instructions short. Shared profile + lean
`AGENTS.md` replaces per-app prose sprawl.

## Related

- [Agent rails and gates](agent-rails.md) — the rails, gates, and the SkillSpector skill gate.
- [Metadata plane](metadata-plane.md) — Ontarch descriptors, policies, registry, and graph.
- [`.agents/profiles/README.md`](../../../../../.agents/profiles/README.md) — the profile contract.

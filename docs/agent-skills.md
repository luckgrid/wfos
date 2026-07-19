# Agent skills — on-demand skill registry

Agent skills are **third-party code**. WfOS treats them like dependencies: the registry holds
**records only** (ids, metadata, scan cache); bodies load **on invocation**, never pre-loaded
into agent context. That keeps token cost down — a skill id in context instead of a full catalog
or body.

## Registry model

| Layer | Location | Role |
|-------|----------|------|
| Authoring | `Workstreams/.agents/skills/*.toml` | Curated records (tracked) |
| Generated | `ontarch/registry/skills.json` | Flattened index (gitignored) |
| Bodies | `$SKILLS_HOME/<body_ref>/SKILL.md` | Installed skill home (default `~/.agents/skills`) |
| Templates | `.agents/skills/templates/*.md` | Repo-local template bodies |

Override the skill home with `SKILLS_HOME` when your layout differs — it is an override point,
not a canonical filesystem layout.

`moon run ontarch:sync` projects each TOML record through `ontarch_skill_record` into
`skills.json`. `moon run ontarch:validate` checks records against `schemas/skill.schema.json`
and enforces:

- `touches` non-empty ⇒ `risks` non-empty
- Profile `allowed_skill_ids` cross-ref valid registry ids
- Loadable skills (listed in a profile) require `scan.status == passed` and non-stale
  `scan.hash == version`

## Profiles and deferred loading

Shared profiles declare which skills may run:

```toml
[skills]
loads_external = true
allowed_skill_ids = ["review", "improve", "qmd", "ponytail"]
```

The profile SkillSpector gate (`loads_external` ⇒ `skillspector_scan` validator) is the trust
prerequisite from [agent-rails.md](agent-rails.md). Per-skill scan records in the registry
complete the gate for loadable skills.

No agent session should start with a skill catalog in its initial prompt — only ids the profile
allows, resolved on call.

## Commands

Report-only tools (agent-safe):

```bash
bin/ontarch skills resolve <id> [--caller PROFILE_ID]   # body path + load-log line
bin/ontarch skills scan <id>                            # record scan (SkillSpector when installed)
bin/ontarch skills map                                  # installed-but-unregistered drift
moon run ontarch:skills-map                             # same as map
```

Load logs append to `packages/ontarch/registry/sessions/skill-loads.jsonl` with
`{skill_id, ts, caller, body_ref, scan_status}`.

Runtime fetch-on-call interception is deferred to the runtime-controller (Takogami) — same boundary as git-push blocking
in [agent-rails.md](agent-rails.md).

## Templates

Common I/O workflows are `kind=template` records pointing at markdown + frontmatter under
`.agents/skills/templates/` (ADR, agent prompt, workflow manifest). Each declares a validator
(`frontmatter` for the POC set). The toolkit RD/PRD/TRD/ARD/SOP family extends the same pattern.

## Fabric patterns

[Fabric](https://github.com/danielmiessler/fabric) patterns are optional `kind=pattern` catalog
entries (`source=fabric`). They stay unscanned and out of `allowed_skill_ids` until installed
and scanned. `ontarch skills resolve` refuses absent bodies with an install-Fabric message.

Fabric and SkillSpector are optional tools — see [tool-catalog.md](tool-catalog.md). Gate
structure lands in the metadata-plane (Ontarch); scanner execution is install-time.

## Graph

`ontarch sync` adds `skill:<id>` nodes and `profile:<p> -can-invoke-> skill:<id>` edges for
profile-declared skill ids alongside existing `profile -selects-> policy` edges.

## Related docs

- [agent-configs.md](agent-configs.md) — shared profiles
- [agent-rails.md](agent-rails.md) — SkillSpector gate + scan cache
- [metadata-plane.md](metadata-plane.md) — registry generation
- [tool-catalog.md](tool-catalog.md) — SkillSpector, Fabric, QMD

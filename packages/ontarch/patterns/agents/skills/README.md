# .agents/skills — curated skill registry

Curated **skill records** (metadata only — never bodies). Bodies resolve on invocation from
`$SKILLS_HOME` (default `~/.agents/skills`, override via env) or, for `kind=template`, from
`$AGENTS_HOME/skills/<body_ref>` if present, else from this pattern's `skills/templates/`.
Skills are third-party code — scan before trusting, the same way you would review a dependency.

Records are tracked TOML under this directory; `ontarch sync` flattens them into
`registry/skills.json` (gitignored). Validate with `moon run ontarch:validate`.

## Record contract

One file per skill/template/pattern. Flat TOML tables only (Ontarch reader subset — no nested
tables, no inline comments).

| Field | Required | Description |
|-------|----------|-------------|
| `id` | yes | Registry id (`^[a-z0-9][a-z0-9._-]*$`) |
| `source` | yes | `factory`, `claude`, `fabric`, `wfos`, or `community` |
| `kind` | yes | `skill`, `template`, `pattern`, or `command` |
| `body_ref` | no | Path key: `$SKILLS_HOME/<body_ref>/SKILL.md` or `templates/…` for `kind=template` |
| `version` | no | Content pin |
| `supported_agent_apps` | no | Apps that may invoke this skill |
| `allowed_contexts` | no | Contexts where the skill applies |
| `touches` | no | `network`, `secrets`, `fs-write`, `fs-read`, `none` |
| `risks` | if `touches` | Human-readable risk notes (required when `touches` non-empty) |
| `validator` | no | Gate id (e.g. `skillspector_scan`, `frontmatter`) |
| `[inputs]` / `[outputs]` | no | Typed parameter map (string types) |
| `[scan]` | no | Cached SkillSpector result (`status`, `hash`, `scanned_at`) |

Example:

```toml
id = "improve"
title = "Improve writing"
source = "community"
kind = "skill"
body_ref = "improve"
version = "1599aa29"
supported_agent_apps = ["factory", "claude", "cursor"]
allowed_contexts = ["build", "docs"]
touches = ["fs-read"]
risks = ["reads repo contents"]
validator = "skillspector_scan"

[inputs]
text = "string"

[outputs]
revised = "markdown"

[scan]
status = "passed"
scanner = "skillspector"
hash = "1599aa29"
scanned_at = "2026-06-26T12:00:00Z"
```

## On-demand loading

Agent context carries **skill IDs only** — not catalogs or bodies. Profiles declare
`[skills] allowed_skill_ids`; resolve a body path at invocation:

```bash
moon run ontarch:skills-map
bin/ontarch skills resolve improve --caller workspace-dev
```

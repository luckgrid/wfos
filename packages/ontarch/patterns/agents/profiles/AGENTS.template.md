# <workspace> agent guide

<!--
LEAN AGENTS.md TEMPLATE. An AGENTS.md is a POINTER, not a manual. Keep it short and directive:
core rules, a may/may-not table, key paths, the profile it runs under, and a skills note.
Detailed commands and architecture belong in README.md and docs/, loaded on demand. Do NOT
restate profile intent here (scope, command allow/block, secrets) — that lives in
.agents/profiles/*.toml and is consumed by every app. Copy this file, fill the
angle-bracket fields, delete this comment.
-->

Keep this file lean. [`README.md`](README.md) and [`docs/`](docs/) are the source of truth for
detailed commands and architecture.

## Core rules

- **<one-line substrate rule>** (e.g. local-first moonrepo; toolchains pinned in `.prototools`).
- **Run from the workspace root** unless a package/app README says otherwise.
- **Native manifests stay authoritative.** Ontarch describes meaning, routing, and policy; it
  never replaces `Cargo.toml`, `package.json`, `mise.toml`, or lockfiles.
- **Stay within the rails.** Agents run under a profile (`.agents/profiles/`); the
  profile's `rails` selects the Ontarch policy that bounds scope, commands, and secrets.

## What agents may / may not do

| Allowed (read-only) | Blocked (human-only) |
|---------------------|----------------------|
| <read-only commands> | <mutating / install / secret commands> |

The profile and the policy it selects are the source of truth — this table is a reminder, not
the rule.

## Key paths

- <toolchain pins / manifests>
- <project graph + tasks>
- Documentation: [`docs/`](docs/)

## Profile

This workspace's default agent profile is **`<profile-id>`**
(`.agents/profiles/<profile-id>.toml`). It declares the scope, command
allow/gate/block lists, secret access, remote-write policy, validators, and output compressor.

## Skills

Agent skills are third-party code. Scan with
[SkillSpector](https://github.com/nvidia/skillspector) before trusting a skill — an unscanned
skill does not load. Skill-loading profiles carry `skillspector_scan` in `required_validators`.

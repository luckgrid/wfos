# Agent rails and gates

WfOS is built for AI agents as first-class operators, but their reach is bounded. The rule is
simple: **detection and reporting are safe; installing software, reading secrets, and editing
dotfiles require a human.**

## How rails work today

[Dust](native-substrate.md) reads the `DUST_AGENT` environment variable. When `DUST_AGENT=1`, only
read-only commands run; mutating ones exit with a non-zero status. The rules live in
[Archon](metadata-plane.md) at `packages/archon/policies/dust.agent.policy.toml`:

```toml
[agent]
env_flag = "DUST_AGENT"
allow = ["doctor", "list", "gen", "env", "version", "help"]
block = ["bootstrap"]

[gates]
no_global_install = true   # no brew/mise installs
no_secret_read = true      # no pass/age/sops invocation
no_shell_mutation = true   # no edits to ~/.zshrc or ~/.config symlinks

[secrets]
block_tools = ["pass", "age", "sops"]   # the no_secret_read hard block
```

So an agent can run `dust doctor`, `dust list`, `dust gen` (dry-run derivation from the
manifest), and `dust env` to understand the machine, but cannot `bootstrap`, install tools,
read secrets, or change dotfiles.

## Publish rails (policy metadata)

Archon also records a **no-agent-git-push** policy at
`packages/archon/policies/no-agent-git-push.policy.toml`. It states that agents may inspect and
stage changes (`git status`, `git diff`, `git add`, `git commit`) but must not publish to a
remote (`git push`, `gh release create`, `gh pr merge`). The policy appears in the generated
project graph (`agent → blocked-by → policy:no-agent-git-push`) and in agent guides.

At Level 0 this is **policy metadata**, not runtime enforcement in Dust: an agent can still invoke
`git push` or `gh` directly on `PATH` unless a future Kraken command router blocks it — the same
boundary as secret tools above. Treat the policy as authoritative intent for agent behavior; OS-level
interception is deferred to Kraken.

## Git command rails (allow / gate / block)

The [`agent-git`](../packages/archon/policies/agent-git.policy.toml) policy declares the
authoritative tiers for agent git usage:

| Tier | Commands | Meaning |
|------|----------|---------|
| **allow** | `git status`, `git diff`, `git log`, `git branch --show-current`, `git worktree list` | inspection — always safe |
| **gate** | `git commit`, `git pull`, `git checkout -b` | human-initiated or elevated; never autonomous |
| **block** | `git push`, `git push --force`, `git reset --hard`, `git clean`, deleting untracked files | destructive or publishing — blocked by default |

Remote writes require **elevated policy plus human approval** (`[remote] writes = "elevated"`).
`agent-git` is the superset git policy; `no-agent-git-push` remains the publish-specific statement,
and both appear in the project graph as `agent → blocked-by → policy`. As with the publish rails,
this is authoritative **intent** and graph metadata at Level 0; OS-level interception of
`git push`/`reset --hard`/`clean` is deferred to the Kraken command router — the same boundary as
the secret tools below.

## Bin/archive mutation rails

The [`agent-bin`](../packages/archon/policies/agent-bin.policy.toml) policy declares the tiers for
bin and lib mutation. It is orthogonal to `agent-git`. Profiles keep their primary `rails` and
select `agent-bin` via an optional `rails_bin` field.

| Tier | Commands | Meaning |
|------|----------|---------|
| **allow** | `archon bin-report`, `archon bin-cleanup --mode report-only`, `du`, `fd`, `stat` | inventory — always safe |
| **gate** | `archon bin-cleanup --mode dry-run` | read-only plan; log intent |
| **block** | `archon bin-cleanup --mode archive`, `… delete-approved`, `rm`, `mv`, `git clean` on `bin/`/`lib/` | mutation — human-only |

Belt-and-suspenders: `archon-bin-cleanup` refuses `archive`/`delete-approved` when `DUST_AGENT=1`,
and at the draft gateway those modes validate arguments then refuse without calling `rm`/`mv`.
Runtime interception of bare `rm`/`mv` on `PATH` is deferred to Kraken — the same boundary as
git and secret rails. See [bin-archive.md](bin-archive.md).

## Worktree isolation

Scoped agents default to a **local branch or worktree**, never the main worktree — isolation
reduces the context surface to the scoped branch and keeps agent work off shared state. Each agent
profile declares an `[isolation]` field (`Workstreams/.agents/profiles/*.toml`):

| Profile | `mode` | `jj` |
|---------|--------|------|
| `workspace-dev` | `worktree` | `opt-in` |
| `agent-safe-maintenance` | `worktree` | `opt-in` |
| `docs-only` | `branch` | `opt-in` |

`mode` ∈ `worktree | branch | main` (the `main` worktree is reserved for elevated profiles).
[jj](https://jj-vcs.github.io/jj/) (Jujutsu) is installed and available for change-oriented
workflows, but it is **opt-in** per profile, never the default — the default isolation is a git
worktree or branch.

## The secret-read hard block

The `no_secret_read` gate is enforced, not advisory. Any code path that would invoke a
secrets-vault tool (`pass`, `age`, `sops`) to resolve a value first calls the
`dust_require_secret_access` guard, which exits non-zero (13) under `DUST_AGENT=1` — so secret
material can never enter agent context. `dust doctor` asserts the rail on every run: it confirms
the blocked tools are marked `agent_safe = false` in the manifest, the policy gate is set, and a
live guard self-test blocks under `DUST_AGENT=1`. A misconfigured rail makes `dust doctor` exit
non-zero. The tiered vault model (interactive `pass` vs file-oriented `sops` + `age`) is documented
in [`packages/dust/dotfiles/SECRETS.md`](../packages/dust/dotfiles/SECRETS.md).

### Enforcement boundary

The guard applies to **Dust-owned code paths** that would resolve a secret value (today:
`dust doctor`, validators, and any future substrate command that shells out to a vault tool).
Agents can still invoke `pass`/`age`/`sops` directly on `PATH` unless a shell wrapper or the
planned runtime-controller command routing blocks them. Level 0 intentionally stops at policy +
manifest flags + doctor assertion; broader OS-level interception is deferred to Kraken.

## Conventions

- **Agent-safe is per-tool.** The Dust manifest marks each tool `agent_safe`; secrets tooling
  (`pass`, `age`, `sops`) is never agent-safe.
- **Apps are build artifacts.** Agents do not start servers or run app builds (`zola serve`,
  long-running dev tasks) without explicit human permission. See [apps.md](apps.md).
- **Scope down by default.** New agent surfaces start blocked and open up deliberately, with
  the policy recorded in Archon.

## MCP exposure (planned)

[Kraken](runtime-controller.md)'s daemon can embed an MCP server (via
[`rmcp`](https://crates.io/crates/rmcp)) that exposes native commands as standardized LLM
tools. Every tool call is checked against the same Archon policies — the MCP surface is a
front door to the rails, not a way around them.

## Skill security (the SkillSpector gate)

Agent skills are third-party code. A skill is **not loaded until it has been scanned** with
[NVIDIA SkillSpector](https://github.com/nvidia/skillspector), a security scanner that detects
vulnerabilities and malicious patterns in agent skills. An unscanned skill is treated exactly like
an unreviewed dependency — it does not run.

This is a **gate, not a convention**. Any agent profile that may load external skills declares
`[skills] loads_external = true` and must list `skillspector_scan` in its `required_validators`
(`Workstreams/.agents/profiles/*.toml`). `archon validate` enforces the pairing: a profile that
loads skills without the `skillspector_scan` validator fails the gate. The `workspace-dev` and
`agent-safe-maintenance` profiles carry it; `docs-only` sets `loads_external = false` and loads no
skills.

The **skills module** adds a per-skill scan record on every curated registry entry. Any skill
listed in a profile's `[skills] allowed_skill_ids` must have `scan.status == "passed"` and a
non-stale `scan.hash == version` or validation fails — a changed skill body invalidates the
cached scan and must be re-scanned (`archon skills scan <id>`) before it can load. Catalogued
skills not in `allowed_skill_ids` warn only. Together, the profile flag and per-skill scan mean
an unscanned or stale skill does not load. See [agent-skills.md](agent-skills.md).

## Agent interface (planned)

A friendly scoped agent/daemon layer (codename Casper) is planned: it observes sessions, reads
system context, suggests actions, and helps operate the runtime controller safely — always
within the rails above, never around them.

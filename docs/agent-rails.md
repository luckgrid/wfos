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
allow = ["doctor", "list", "env", "version", "help"]
block = ["bootstrap"]

[gates]
no_global_install = true   # no brew/mise installs
no_secret_read = true      # no pass/age/sops invocation
no_shell_mutation = true   # no edits to ~/.zshrc or ~/.config symlinks
```

So an agent can run `dust doctor`, `dust list`, and `dust env` to understand the machine, but
cannot `bootstrap`, install tools, read secrets, or change dotfiles.

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

## Skill security

Agent skills are third-party code. Before a skill is trusted, scan it with
[NVIDIA SkillSpector](https://github.com/nvidia/skillspector), a security scanner that detects
vulnerabilities and malicious patterns in agent skills. Treat an unscanned skill the same way
you would treat an unreviewed dependency.

## Agent interface (planned)

A friendly scoped agent/daemon layer (codename Casper) is planned: it observes sessions, reads
system context, suggests actions, and helps operate the runtime controller safely — always
within the rails above, never around them.

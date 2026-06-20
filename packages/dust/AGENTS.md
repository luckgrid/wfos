# Dust agent guide

Read-only by default. [`README.md`](README.md) and
[`../../docs/native-substrate.md`](../../docs/native-substrate.md) are the source of truth.

## Rails

`dust` reads `DUST_AGENT`. With `DUST_AGENT=1`:

- **Allowed:** `doctor`, `list`, `env`, `version`, `help`.
- **Blocked:** `bootstrap` (and all installs, secret reads, dotfile edits) — exits non-zero.

The policy is `../archon/policies/dust.agent.policy.toml`; gates: `no_global_install`,
`no_secret_read`, `no_shell_mutation`.

## Do / don't

- Do run `dust doctor` to understand machine readiness and `dust list` to see modules/tools.
- Do read the manifest (`manifest/dust.tools.toml`) — it is the source of truth.
- Don't install tools, run `bootstrap`, read secrets (`pass`/`age`/`sops`), or edit
  `~/.zshrc` / `~/.config`. Those require a human.

## Editing the substrate

- Add or change a tool only in `manifest/dust.tools.toml`; derived artifacts (Brewfile, the
  Archon registry, shell activations) follow from it.
- Keep scripts POSIX/bash and `shellcheck`-clean. Preserve the guarded-activation pattern in
  `config/shell/dust.zsh` (safe to source when a tool is absent).

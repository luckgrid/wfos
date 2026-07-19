# Panoply agent guide

Read-only by default. [`README.md`](README.md) and
[`../../docs/native-toolchain.md`](../../docs/native-toolchain.md) are the source of truth.

## Rails

`panoply` reads `PANOPLY_AGENT`. With `PANOPLY_AGENT=1`:

- **Allowed:** `doctor` (incl. `--json`), `list`, `gen` (dry-run artifact derivation),
  `env` (incl. `--json`/`--shell`), `version`, `help`.
- **Blocked:** `bootstrap` (and all installs, secret reads, dotfile edits) — exits non-zero.

The policy is `../ontarch/policies/panoply.agent.policy.toml`; gates: `no_global_install`,
`no_secret_read`, `no_shell_mutation`.

## Do / don't

- Do run `panoply doctor` to understand machine readiness and `panoply list` to see modules/tools.
- Do read the manifest (`manifest/panoply.tools.toml`) — it is the source of truth.
- Don't install tools, run `bootstrap`, read secrets (`pass`/`age`/`sops`), or edit
  `~/.zshrc` / `~/.config`. Those require a human.

## Editing the substrate

- Add or change a tool only in `manifest/panoply.tools.toml`; derived artifacts (Brewfile, the
  Ontarch registry, shell activations) follow from it.
- Keep scripts POSIX/bash and `shellcheck`-clean. Preserve the guarded-activation pattern in
  `config/shell/panoply.zsh` (safe to source when a tool is absent).

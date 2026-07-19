# Secrets & vaults — tiered model

The secrets layer of the WfOS machine-config substrate. It enforces a strict **separation of
concerns**: interactive credentials and git-checked configuration live in different vaults, and
**agents can never read either**. The cheapest token is the one never loaded — hard-blocking
secret reads means secret material cannot enter an agent's context.

> **Draft posture:** this directory defines the rails; it does not exercise them. Nothing here
> stores a secret *value*. Secret values are resolved only at chezmoi **apply** time, only on a
> profile that permits it, and only by a human.

## Two vaults, one concern each

The contract is machine-readable in [`.chezmoidata/vaults.toml`](.chezmoidata/vaults.toml). No
concern is served by two vaults.

```txt
[ Substrate ]
├── Interactive keys / CLI logins ──> pass        (GnuPG store)
└── Repo config files ─────────────> sops + age   (encrypted YAML/JSON values)
```

| Vault | Concern | Holds | Backend | Agent-readable |
|-------|---------|-------|---------|----------------|
| `pass` | interactive | CLI logins, personal API keys, script env queries | GnuPG | no |
| `sops` + `age` | files | values inside git-checked config files | age (X25519) | no |

- **`pass`** is the root CLI wallet for interactive workflows. It is a key-value record store, not
  a tool for system configuration files.
- **`sops` + `age`** is used **exclusively** for files checked into git. sops encrypts only the
  *values*, leaving keys and structure readable so diffs stay mergeable; `age` is the modern
  backend (X25519, no agent daemon) in place of PGP.
- **No raw secret is ever committed to a public config path.** A sops file commits ciphertext, not
  plaintext; `pass` material never lands in a tracked file at all.

## Agent hard-block

`pass`, `age`, and `sops` are marked `agent_safe = false` in the Panoply manifest — that flag is the
source of truth. The Ontarch policy
([`../../ontarch/policies/panoply.agent.policy.toml`](../../ontarch/policies/panoply.agent.policy.toml))
makes `no_secret_read = true` authoritative and lists the blocked tools. Under `PANOPLY_AGENT=1`, the
`panoply_require_secret_access` guard exits non-zero (13), and `panoply doctor` asserts the rail
(manifest flags + policy gate + a live guard self-test). Detection of a tool's *presence* is safe;
*invoking* it to read a value is blocked.

### Enforcement boundary

The guard applies to **Panoply-owned code paths** that would resolve a secret value (validators,
`panoply doctor`, and future substrate commands). Agents can still invoke `pass`/`age`/`sops`
directly on `PATH` unless a shell wrapper or the planned runtime-controller command routing
blocks them. Level 0 stops at policy + manifest flags + doctor assertion.

## chezmoi integration

chezmoi resolves secrets at **apply** time only, never storing values in source:

- **pass references** live under [`private_dot_config/wfos-secrets/`](private_dot_config/wfos-secrets/)
  (mode `0600`). The template body is guarded on the profile's `secrets` flag; the `agent-safe` and
  `headless-dev` profiles exclude the whole `secrets` category via
  [`.chezmoiignore.tmpl`](.chezmoiignore.tmpl), so `chezmoi diff` for those profiles never even
  renders the file — no secret reference is resolved.
- **sops + age files** follow [`../secrets/README.md`](../secrets/README.md). The committed
  [`sample.config.enc.yaml`](../secrets/sample.config.enc.yaml) is a fixture-only ciphertext example;
  [`sample.config.yaml`](../secrets/sample.config.yaml) holds plaintext structure for edits.

### Human smoke (when chezmoi is installed)

```bash
export WFOS_PROFILE=agent-safe
chezmoi diff --source "$(pwd)"   # from packages/panoply/dotfiles — must NOT list wfos-secrets
```

`local-macos-full` may resolve pass references on `chezmoi apply` only (human-gated).

## Human bootstrap (not exercised in validators)

- Initialize `pass` (`pass init`) and your GnuPG identity before relying on pass-backed templates.
- Install chezmoi (`brew install chezmoi` or `panoply bootstrap` / dotfiles module) before live diff.
- Provision a production age recipient before replacing the fixture key in `.sops.yaml`.

## gitleaks (leak gate)

[`gitleaks`](https://github.com/gitleaks/gitleaks) (MIT) is in the Panoply manifest `secrets` module
and the candidate install set (Brewfile). It scans staged/committed files for leaked secrets; the
pre-commit hook that runs it is wired by the git-hygiene module. Scanning is read-only reporting, so
gitleaks is `agent_safe = true` (unlike the vaults, it never exposes secret values into context).

## Validate (dry-run gate, no secret reads)

```bash
bin/validate-secrets.sh        # vault non-overlap, agent hard-block proof, chezmoi refs, gitleaks
```

From the workspace root: `moon run panoply:validate-secrets`. The gate proves the `PANOPLY_AGENT=1`
block (exit 13) and asserts manifest/policy consistency; it never invokes `pass`/`age`/`sops` to
read a value, and the secret-reference template is static-checked only.

## Related

- [`.chezmoidata/vaults.toml`](.chezmoidata/vaults.toml) — machine-readable vault contract
- [`ROUTING.md`](ROUTING.md) — config routing rules (no app config holds secrets)
- [`../../ontarch/policies/panoply.agent.policy.toml`](../../ontarch/policies/panoply.agent.policy.toml) — agent rails + `no_secret_read`
- [`../../../docs/agent-rails.md`](../../../docs/agent-rails.md) — agent rails and gates

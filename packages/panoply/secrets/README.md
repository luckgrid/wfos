# sops + age fixtures — the `files` vault

This directory holds the **files** vault example: configuration checked into git whose *values*
are encrypted with [sops](https://github.com/getsops/sops) using an [age](https://age-encryption.org/)
recipient. It is the counterpart to the interactive `pass` vault (see
[`../dotfiles/SECRETS.md`](../dotfiles/SECRETS.md)).

> **Fixture-only key material.** The committed [`sample.config.enc.yaml`](sample.config.enc.yaml)
> uses a **repo-local test age keypair** — not a production secret. Plaintext structure lives in
> [`sample.config.yaml`](sample.config.yaml) for editing; only the `.enc.yaml` file is meant for git.

## Files

| File | Role |
|------|------|
| [`.sops.yaml`](.sops.yaml) | creation rules: `.enc.*` files encrypt to the fixture age recipient |
| [`sample.config.yaml`](sample.config.yaml) | plaintext **structure** only — keys + fake placeholders (edit source) |
| [`sample.config.enc.yaml`](sample.config.enc.yaml) | sops-encrypted values — safe to commit (ciphertext only) |

## Fixture-only age key (not production)

Public recipient (in `.sops.yaml`):

```txt
age1sypm2y50ryrz80fq70gpavmrgp4wrscru8zz9v6yuats4rc393sq54ftfs
```

Paired test private key (for local decrypt/regeneration only — **do not reuse in production**):

```txt
AGE-SECRET-KEY-1CRECGV6QKCCRP8USMWQ8Q27M59YC9SYEE68E7ENRS4CM94XNY6KS93R0EG
```

## Recipe

```bash
# Regenerate ciphertext after editing sample.config.yaml (fixture recipient):
sops --encrypt --age age1sypm2y50ryrz80fq70gpavmrgp4wrscru8zz9v6yuats4rc393sq54ftfs \
  sample.config.yaml > sample.config.enc.yaml

# Decrypt is HUMAN-only (agents are hard-blocked: no_secret_read):
SOPS_AGE_KEY_FILE=<path-to-fixture-key> sops --decrypt sample.config.enc.yaml
```

Production hosts: generate your own key (`age-keygen`), replace the recipient in `.sops.yaml`,
and never commit the private key.

## Why tiered

`pass` is for interactive CLI credentials; `sops` + `age` is for git-checked files. Keeping them
separate means agent-readable config carries only structure, never values, and the secret-read
hard block (`PANOPLY_AGENT=1`) keeps `pass`/`age`/`sops` invocation out of agent context entirely.

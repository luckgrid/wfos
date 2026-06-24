# sops + age fixtures — the `files` vault

This directory holds the **files** vault example: configuration checked into git whose *values*
are encrypted with [sops](https://github.com/getsops/sops) using an [age](https://age-encryption.org/)
recipient. It is the counterpart to the interactive `pass` vault (see
[`../dotfiles/SECRETS.md`](../dotfiles/SECRETS.md)).

> **No secret values live here.** [`sample.config.yaml`](sample.config.yaml) is a structure-only
> example with fake placeholders. A sops-encrypted file commits *ciphertext*, not plaintext, so it
> is safe to track — but encryption is **deferred** until an age recipient key is provisioned (a
> human step). This session defines the rails; it does not generate key material.

## Files

| File | Role |
|------|------|
| [`.sops.yaml`](.sops.yaml) | creation rules: which files encrypt to which age recipient (placeholder) |
| [`sample.config.yaml`](sample.config.yaml) | plaintext **structure** only — keys + fake values |

## Recipe (deferred — run on a host with a provisioned age key)

```bash
# 1. Generate / locate an age key (human step; key stays out of git).
age-keygen -o ~/.config/wfos-secrets/age/keys.txt   # prints the public recipient

# 2. Put the real recipient (age1...) into .sops.yaml, replacing the placeholder.

# 3. Encrypt values in place -> a committable, diff-friendly ciphertext file.
sops --encrypt sample.config.yaml > sample.config.enc.yaml

# 4. Decrypt is a HUMAN-only action (agents are hard-blocked: no_secret_read).
sops --decrypt sample.config.enc.yaml
```

## Why tiered

`pass` is for interactive CLI credentials; `sops` + `age` is for git-checked files. Keeping them
separate means agent-readable config carries only structure, never values, and the secret-read
hard block (`DUST_AGENT=1`) keeps `pass`/`age`/`sops` invocation out of agent context entirely.

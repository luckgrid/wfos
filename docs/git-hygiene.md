# Git hygiene

Git is the legible distribution layer. Repo-local hooks catch problems before they land, and clean,
parseable history compresses better and needs less agent reasoning. Hooks are **per-repo, never
global** — each docs/config repo carries its own `.pre-commit-config.yaml`.

## Hook baseline (pre-commit)

The reference config lives at the workspace root ([`.pre-commit-config.yaml`](../.pre-commit-config.yaml))
and is driven by [pre-commit](https://pre-commit.com/):

| Hook | Source | Catches |
|------|--------|---------|
| `trailing-whitespace`, `end-of-file-fixer` | pre-commit-hooks | whitespace / newline noise |
| `check-added-large-files` | pre-commit-hooks | oversized blobs (`--maxkb=1024`) |
| `check-merge-conflict` | pre-commit-hooks | leftover conflict markers |
| `detect-private-key` | pre-commit-hooks | committed private keys |
| `gitleaks` | local (installed binary) | staged secrets (see below) |
| `shellcheck` | local (installed binary) | shell script bugs |
| `biome check` | local (installed binary) | JS/TS/JSON lint + format |
| `shfmt` | pinned repo hook | shell formatting |
| `markdownlint` | pinned repo hook | Markdown lint |

Local hooks (`gitleaks`, `shellcheck`, `biome`) run the tools already on the host — no network, no
cloned environments. The pinned repo hooks (`shfmt`, `markdownlint`) build isolated environments the
first time they run; the host does not need those tools on `PATH`.

### Adoption

Hooks are installed per repo by a human (installing writes `.git/hooks/`, a mutation agents do not
perform):

```bash
cp .pre-commit-config.yaml <other-repo>/    # reuse the baseline in a docs/config repo
pre-commit install                          # wire the hooks (human step)
pre-commit run --all-files                  # first full pass
```

Agents may **validate** the config without installing anything:

```bash
pre-commit validate-config .pre-commit-config.yaml
```

### Decision: pre-commit first, lefthook deferred

[pre-commit](https://pre-commit.com/) is the baseline because it is already installed and its
managed-environment model keeps hooks reproducible across repos. [lefthook](https://lefthook.dev/)
is faster but is not installed, and installing tools is a human-gated action. **Decision:** stay on
pre-commit; revisit lefthook only if hook latency becomes a real problem. This keeps hooks per-repo
and avoids a global hook manager.

## Secret scanning (gitleaks)

No secret reaches a commit. [gitleaks](https://github.com/gitleaks/gitleaks) runs two ways:

- **Pre-commit gate** — the local `gitleaks` hook runs `gitleaks git --staged` on every commit; a
  blocked secret is reported with its file and line and the commit is rejected.
- **Scan routine** — `moon run archon:secrets-scan` runs a whole-repo, history-aware scan
  (`archon secrets-scan detect`). A periodic run is a workflow-automation concern.

Both read the baseline [`.gitleaks.toml`](../.gitleaks.toml) (the built-in ruleset plus a low-noise
allowlist for generated registry output and session logs). The routine is **report-only**: it
scans, it never stages or writes tracked files, and it performs no remote operation. Scan a
non-git directory or export with `archon secrets-scan dir <path>`.

## Conventional commits

Commit subjects follow a lightweight convention so history stays parseable and compressible:

```txt
<type>[(scope)][!]: <description>

type ∈ feat | fix | docs | chore | refactor | test
```

Examples: `feat(archon): add read-only polyrepo scan report`, `fix: correct default-branch
detection`, `docs: document the gitleaks gate`.

The check is a **dependency-free** `commit-msg` hook
([`packages/archon/hooks/commit-msg`](../packages/archon/hooks/commit-msg), bash regex — no
commitlint, cog, or czg required), wired into the pre-commit config at the `commit-msg` stage.
Merge, revert, and fixup subjects pass through.

### Decision: lightweight now, enforced later

The convention is **lightweight**: the regex hook is the only enforcement. Adopting a full
enforcer (`commitlint`, `cog`, or `czg`) is deferred until history-based automation
(changelogs, release notes) needs it — none of those tools is installed, and installing them is
human-gated. Keeping `git log --oneline` grammar-consistent already lets RTK compress history and
lets agents parse it without a heavier toolchain.

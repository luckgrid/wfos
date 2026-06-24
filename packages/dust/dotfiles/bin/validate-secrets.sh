#!/usr/bin/env bash
# validate-secrets.sh — dry-run gate for the WfOS secrets & vaults rails.
#
# Checks the secrets layer WITHOUT reading or resolving any secret value:
#   1. vaults      — tiered contract: one concern per vault (no overlap), all agent_readable=false
#   2. hard-block  — policy no_secret_read=true; blocked tools agent_safe=false; DUST_AGENT=1
#                    guard exits 13 (live self-test of the guard, NOT a real secret read)
#   3. chezmoi     — pass-reference template guards on the profile `secrets` flag; the secrets
#                    target is excluded for agent-safe; no secret-looking literal in source/fixtures
#   4. gitleaks    — present in the manifest secrets module + the candidate install set (Brewfile)
#   5. docs        — SECRETS.md present
#
# The secret-reference template is STATIC-checked only; it is never passed to chezmoi
# execute-template (that would call `pass`). Exit 0 = pass, 1 = fail. Agent-safe / read-only.
set -uo pipefail

BIN="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$BIN/.." && pwd)"
DUST_PKG="$(cd "$SRC/.." && pwd)"
MANIFEST="$DUST_PKG/manifest/dust.tools.toml"
BREWFILE="$DUST_PKG/config/Brewfile"
POLICY="$DUST_PKG/../archon/policies/dust.agent.policy.toml"
COMMON="$DUST_PKG/lib/common.sh"
MANIFEST_LIB="$DUST_PKG/lib/manifest.sh"
VAULTS="$SRC/.chezmoidata/vaults.toml"
PROFILES="$SRC/.chezmoidata/profiles.toml"
IGNORE="$SRC/.chezmoiignore.tmpl"
SECRET_TMPL="$SRC/private_dot_config/wfos-secrets/env.sh.tmpl"
FIXTURES="$DUST_PKG/secrets"

# shellcheck source=../../lib/common.sh
source "$COMMON"
# shellcheck source=../../lib/manifest.sh
source "$MANIFEST_LIB"

fail=0
note() { printf '  %s\n' "$*"; }
ok()   { printf 'PASS %s\n' "$*"; }
bad()  { printf 'FAIL %s\n' "$*"; fail=1; }

# agent_safe value (manifest field 6) for a tool id, or MISSING.
agent_safe_of() {
  dust_manifest_tsv | awk -F'\037' -v id="$1" '$2==id {print $6; f=1} END{if(!f) print "MISSING"}'
}

echo "== secrets & vaults validation: $DUST_PKG =="

# 1. vault contract ----------------------------------------------------------
echo "[1/5] vault contract"
if [ -f "$VAULTS" ]; then
  for v in pass sops; do
    grep -q "^\[vaults.$v\]" "$VAULTS" && note "vault defined: $v" || bad "vault missing: $v"
  done
  n_vaults=$(grep -cE '^\[vaults\.' "$VAULTS")
  concerns=$(grep -E '^concern[[:space:]]*=' "$VAULTS" | sed -E 's/.*=[[:space:]]*"?([^"]*)"?.*/\1/' | tr -d ' ')
  n_concern=$(printf '%s\n' "$concerns" | grep -c .)
  n_concern_u=$(printf '%s\n' "$concerns" | sort -u | grep -c .)
  if [ "$n_concern" = "$n_concern_u" ]; then
    note "no concern overlap ($n_concern_u distinct concerns across $n_vaults vaults)"
  else
    bad "concern overlap: $n_concern concerns but only $n_concern_u distinct"
  fi
  n_readable_false=$(grep -cE '^agent_readable[[:space:]]*=[[:space:]]*false' "$VAULTS")
  if [ "$n_readable_false" = "$n_vaults" ]; then
    note "all $n_vaults vaults agent_readable=false"
  else
    bad "not every vault is agent_readable=false ($n_readable_false/$n_vaults)"
  fi
  [ "$fail" -eq 0 ] && ok "tiered vault contract (no overlap, agents excluded)"
else
  bad ".chezmoidata/vaults.toml missing"
fi

# 2. agent hard-block --------------------------------------------------------
echo "[2/5] agent secret-read hard-block"
if [ -f "$POLICY" ] && grep -Eq '^[[:space:]]*no_secret_read[[:space:]]*=[[:space:]]*true' "$POLICY"; then
  note "policy no_secret_read = true"
else
  bad "policy no_secret_read not true (or policy missing)"
fi
if [ -f "$POLICY" ]; then
  block_tools=$(grep -E '^[[:space:]]*block_tools[[:space:]]*=' "$POLICY" | sed -E 's/.*\[(.*)\].*/\1/; s/[",]/ /g')
else
  block_tools=""
fi
[ -n "$block_tools" ] && note "policy block_tools: $block_tools" || bad "policy block_tools missing"
for t in $block_tools; do
  as=$(agent_safe_of "$t")
  [ "$as" = "false" ] && note "manifest $t agent_safe=false" || bad "manifest $t agent_safe=$as (must be false)"
done
# live guard self-test: must exit 13 under DUST_AGENT=1 (does NOT read a secret).
( export DUST_AGENT=1; dust_require_secret_access pass ) >/dev/null 2>&1
guard_rc=$?
[ "$guard_rc" -eq 13 ] && note "DUST_AGENT=1 guard blocks (exit 13)" || bad "DUST_AGENT=1 guard did not block (exit $guard_rc)"
# and it must NOT block for a human (DUST_AGENT unset/0).
( unset DUST_AGENT; dust_require_secret_access pass ) >/dev/null 2>&1
human_rc=$?
[ "$human_rc" -eq 0 ] && note "human (DUST_AGENT unset) not blocked" || bad "guard blocked a human (exit $human_rc)"
[ "$fail" -eq 0 ] && ok "no_secret_read enforced (policy + manifest + live guard)"

# 3. chezmoi secret integration ----------------------------------------------
echo "[3/5] chezmoi secret references"
if [ -f "$SECRET_TMPL" ]; then
  grep -q '\$p.secrets' "$SECRET_TMPL" && note "secret template guards on profile secrets flag" \
    || bad "secret template not guarded on \$p.secrets"
  grep -q 'pass "' "$SECRET_TMPL" && note "references pass vault via chezmoi pass function" \
    || bad "secret template has no pass reference"
else
  bad "missing secret-reference template: ${SECRET_TMPL#"$DUST_PKG"/}"
fi
# the secrets category must be excluded for the agent-safe profile (so diff never resolves it).
if [ -f "$PROFILES" ]; then
  agent_block=$(awk '/^\[profiles.agent-safe\]/{f=1;next} /^\[/{f=0} f' "$PROFILES")
  printf '%s' "$agent_block" | grep -q '"secrets"' && note "agent-safe excludes the secrets category" \
    || bad "agent-safe does not exclude secrets (diff could resolve a reference)"
fi
[ -f "$IGNORE" ] && grep -q 'wfos-secrets' "$IGNORE" && note ".chezmoiignore.tmpl maps secrets -> .config/wfos-secrets" \
  || bad ".chezmoiignore.tmpl does not map the secrets category to wfos-secrets"
# no secret-looking literal in the templates or fixtures.
secret_re='BEGIN[[:space:]]+(RSA|OPENSSH|PGP|EC)[[:space:]]+PRIVATE KEY|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{30,}|xox[baprs]-[A-Za-z0-9-]+|-----BEGIN'
if grep -REn "$secret_re" "$SRC/private_dot_config" "$FIXTURES" >/dev/null 2>&1; then
  bad "secret-looking literal found:"; grep -REn "$secret_re" "$SRC/private_dot_config" "$FIXTURES" | sed 's/^/    /'
else
  note "no secret-looking literals in templates or fixtures (referenced, not stored)"
fi
# live render is deferred when chezmoi is absent; never render the secret template here.
if command -v chezmoi >/dev/null 2>&1; then
  note "chezmoi present — live agent-safe diff is a human step (not run: would touch \$HOME/secrets)"
else
  note "chezmoi NOT installed — live 'chezmoi diff' DEFERRED (install via dust bootstrap)"
fi
[ "$fail" -eq 0 ] && ok "chezmoi secret references (guarded; agent-safe excluded; deferred render)"

# 4. gitleaks prerequisite ---------------------------------------------------
echo "[4/5] gitleaks prerequisite"
grep -q '^id = "gitleaks"' "$MANIFEST" && note "gitleaks in manifest" || bad "gitleaks missing from manifest"
[ "$(agent_safe_of gitleaks)" = "true" ] && note "gitleaks agent_safe=true (scanner/reporting)" \
  || note "gitleaks agent_safe not true (review)"
grep -q '"gitleaks"' "$BREWFILE" && note "gitleaks in Brewfile (candidate install set)" \
  || bad "gitleaks missing from Brewfile"
[ "$fail" -eq 0 ] && ok "gitleaks added to manifest + candidate set"

# 5. docs --------------------------------------------------------------------
echo "[5/5] docs"
[ -f "$SRC/SECRETS.md" ] && note "SECRETS.md present" || bad "SECRETS.md missing"
[ -f "$FIXTURES/README.md" ] && note "secrets/README.md (sops+age recipe) present" || bad "secrets/README.md missing"
[ "$fail" -eq 0 ] && ok "docs present"

echo
if [ "$fail" -eq 0 ]; then echo "RESULT: PASS (no secret read or resolution)"; else echo "RESULT: FAIL"; fi
exit "$fail"

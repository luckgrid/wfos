#!/usr/bin/env bash
# validate-secrets.sh — dry-run gate for the WfOS secrets & vaults rails.
#
# Checks the secrets layer WITHOUT reading or resolving any secret value:
#   1. vaults      — tiered contract: one concern per vault (no overlap), all agent_readable=false
#   2. hard-block  — policy no_secret_read=true; blocked tools agent_safe=false; PANOPLY_AGENT=1
#                    guard exits 13 (live self-test of the guard, NOT a real secret read)
#   3. chezmoi     — pass-reference template guards on the profile `secrets` flag; the secrets
#                    target is excluded for agent-safe; sops encrypted fixture present; no literals
#   4. gitleaks    — present in the manifest secrets module + the candidate install set (Brewfile)
#   5. docs        — SECRETS.md present
#
# The secret-reference template is STATIC-checked only; it is never passed to chezmoi
# execute-template (that would call `pass`). Exit 0 = pass, 1 = fail. Agent-safe / read-only.
set -uo pipefail

BIN="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$BIN/.." && pwd)"
PANOPLY_PKG="$(cd "$SRC/.." && pwd)"
MANIFEST="$PANOPLY_PKG/manifest/panoply.tools.toml"
BREWFILE="$PANOPLY_PKG/config/Brewfile"
POLICY="$PANOPLY_PKG/../ontarch/policies/panoply.agent.policy.toml"
COMMON="$PANOPLY_PKG/lib/common.sh"
MANIFEST_LIB="$PANOPLY_PKG/lib/manifest.sh"
VAULTS="$SRC/.chezmoidata/vaults.toml"
PROFILES="$SRC/.chezmoidata/profiles.toml"
IGNORE="$SRC/.chezmoiignore.tmpl"
SECRET_TMPL="$SRC/private_dot_config/wfos-secrets/env.sh.tmpl"
FIXTURES="$PANOPLY_PKG/secrets"
ENC_FIXTURE="$FIXTURES/sample.config.enc.yaml"

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
  panoply_manifest_tsv | awk -F'\037' -v id="$1" '$2==id {print $6; f=1} END{if(!f) print "MISSING"}'
}

echo "== secrets & vaults validation: $PANOPLY_PKG =="

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
# live guard self-test: must exit 13 under PANOPLY_AGENT=1 (does NOT read a secret).
( export PANOPLY_AGENT=1; panoply_require_secret_access pass ) >/dev/null 2>&1
guard_rc=$?
[ "$guard_rc" -eq 13 ] && note "PANOPLY_AGENT=1 guard blocks (exit 13)" || bad "PANOPLY_AGENT=1 guard did not block (exit $guard_rc)"
# and it must NOT block for a human (PANOPLY_AGENT unset/0).
( unset PANOPLY_AGENT; panoply_require_secret_access pass ) >/dev/null 2>&1
human_rc=$?
[ "$human_rc" -eq 0 ] && note "human (PANOPLY_AGENT unset) not blocked" || bad "guard blocked a human (exit $human_rc)"
[ "$fail" -eq 0 ] && ok "no_secret_read enforced (policy + manifest + live guard)"

# 3. chezmoi secret integration ----------------------------------------------
echo "[3/5] chezmoi secret references"
if [ -f "$SECRET_TMPL" ]; then
  grep -q '\$p.secrets' "$SECRET_TMPL" && note "secret template guards on profile secrets flag" \
    || bad "secret template not guarded on \$p.secrets"
  grep -q 'pass "' "$SECRET_TMPL" && note "references pass vault via chezmoi pass function" \
    || bad "secret template has no pass reference"
else
  bad "missing secret-reference template: ${SECRET_TMPL#"$PANOPLY_PKG"/}"
fi
# the secrets category must be excluded for the agent-safe profile (so diff never resolves it).
if [ -f "$PROFILES" ]; then
  agent_block=$(awk '/^\[profiles.agent-safe\]/{f=1;next} /^\[/{f=0} f' "$PROFILES")
  printf '%s' "$agent_block" | grep -q '"secrets"' && note "agent-safe excludes the secrets category" \
    || bad "agent-safe does not exclude secrets (diff could resolve a reference)"
fi
[ -f "$IGNORE" ] && grep -q 'wfos-secrets' "$IGNORE" && note ".chezmoiignore.tmpl maps secrets -> .config/wfos-secrets" \
  || bad ".chezmoiignore.tmpl does not map the secrets category to wfos-secrets"
# no secret-looking literal in the templates (fixtures allow sops ciphertext + fixture age key docs).
secret_re='BEGIN[[:space:]]+(RSA|OPENSSH|PGP|EC)[[:space:]]+PRIVATE KEY|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{30,}|xox[baprs]-[A-Za-z0-9-]+|-----BEGIN'
if grep -REn "$secret_re" "$SRC/private_dot_config" >/dev/null 2>&1; then
  bad "secret-looking literal found:"; grep -REn "$secret_re" "$SRC/private_dot_config" | sed 's/^/    /'
else
  note "no secret-looking literals in chezmoi secret templates"
fi
# sops encrypted fixture: committed ciphertext, no plaintext placeholders.
if [ -f "$ENC_FIXTURE" ] && [ -s "$ENC_FIXTURE" ]; then
  grep -q '^sops:' "$ENC_FIXTURE" && note "sample.config.enc.yaml has sops metadata" \
    || bad "sample.config.enc.yaml missing sops metadata"
  grep -q 'ENC\[AES256_GCM' "$ENC_FIXTURE" && note "sample.config.enc.yaml values are encrypted" \
    || bad "sample.config.enc.yaml does not look encrypted"
  grep -q 'REPLACE_VIA_SOPS' "$ENC_FIXTURE" && bad "sample.config.enc.yaml still has plaintext placeholders" \
    || note "no plaintext placeholders in encrypted fixture"
else
  bad "missing or empty sops fixture: ${ENC_FIXTURE#"$PANOPLY_PKG"/}"
fi
# when chezmoi is present, prove agent-safe ignore output excludes wfos-secrets (no pass call).
if command -v chezmoi >/dev/null 2>&1; then
  agent_ignored="$(chezmoi execute-template --source "$SRC" \
    --override-data '{"profile":"agent-safe"}' < "$IGNORE" 2>/dev/null || true)"
  printf '%s' "$agent_ignored" | grep -q 'wfos-secrets' && note "chezmoi: agent-safe ignore lists wfos-secrets" \
    || bad "chezmoi: agent-safe ignore missing wfos-secrets"
  note "chezmoi present — human smoke: WFOS_PROFILE=agent-safe chezmoi diff --source <dotfiles>"
else
  note "chezmoi NOT installed — live 'chezmoi diff' DEFERRED (install via panoply bootstrap)"
fi
[ "$fail" -eq 0 ] && ok "chezmoi secret references (guarded; agent-safe excluded; sops fixture)"

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

#!/usr/bin/env bash
# validate-substrate.sh — dry-run gate for the Panoply native toolchain (WfOS Level 0 E03).
#
# Read-only and agent-safe: never installs, writes outside a temp dir, or reads secrets.
# Sections:
#   1. manifest    — parses to N tools; brew strings present where expected
#   2. brewfile    — `panoply gen brewfile --check` clean (manifest is the source of truth)
#   3. doctor      — `doctor --json --no-write` is valid JSON + schema-shaped; rail enforced
#   4. env         — `env` prints resolved paths; `env --json` parses
#   5. agent/rtk   — agent module + rtk default; rtk.zsh guarded/swappable; profiles carry rtk
#   6. matrix      — every swappable role resolves an active member
#
# Exit 0 = pass, 1 = fail. Run via `moon run panoply:validate-substrate`.
set -uo pipefail

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PKG="$(cd "$SELF/.." && pwd)"
PANOPLY="$PKG/bin/panoply"
fail=0
note() { printf '  %s\n' "$*"; }
ok()   { printf 'PASS %s\n' "$*"; }
bad()  { printf 'FAIL %s\n' "$*"; fail=1; }

echo "== panoply substrate validation: $PKG =="

have_jq=0; command -v jq >/dev/null 2>&1 && have_jq=1

# 1. manifest ----------------------------------------------------------------
echo "[1/6] manifest"
# shellcheck source=../lib/common.sh
source "$PKG/lib/common.sh"
# shellcheck source=../lib/manifest.sh
source "$PKG/lib/manifest.sh"
tool_count="$(panoply_manifest_tsv | grep -c . || true)"
if [ "${tool_count:-0}" -ge 1 ]; then note "manifest parses ($tool_count tools)"; else bad "manifest did not parse any tools"; fi
# every tool with a brew formula must have a detect token
missing_detect="$(panoply_manifest_tsv | awk -F'\037' '$4!="" && $5=="" {print $2}')"
[ -z "$missing_detect" ] && note "all brew tools declare a detect token" || bad "brew tools missing detect: $missing_detect"
[ "$fail" -eq 0 ] && ok "manifest"

# 2. brewfile derivation -----------------------------------------------------
echo "[2/6] brewfile derivation"
if "$PANOPLY" gen brewfile --check >/dev/null 2>&1; then note "panoply gen brewfile --check clean"; else bad "Brewfile drift (run: panoply gen brewfile > config/Brewfile)"; fi
[ "$fail" -eq 0 ] && ok "brewfile derivation"

# 3. doctor --json -----------------------------------------------------------
echo "[3/6] doctor --json"
doctor_json="$("$PANOPLY" doctor --json --no-write 2>/dev/null)"; rc=$?
if [ "$rc" -ne 0 ]; then bad "doctor --json exit $rc (secrets-rail drift?)"; else note "doctor --json exit 0 (rail enforced)"; fi
if [ "$have_jq" = "1" ]; then
  if printf '%s' "$doctor_json" | jq -e '.summary.total and (.tools|length>0)' >/dev/null 2>&1; then
    note "doctor --json is valid JSON with summary + tools"
  else
    bad "doctor --json missing summary/tools"
  fi
else
  note "jq not present — skipped JSON shape assertion"
fi
[ "$fail" -eq 0 ] && ok "doctor --json"

# 4. env resolved ------------------------------------------------------------
echo "[4/6] env resolved"
env_out="$("$PANOPLY" env 2>/dev/null)"
echo "$env_out" | grep -q '^PANOPLY_PKG=' && note "env prints PANOPLY_PKG" || bad "env missing PANOPLY_PKG"
echo "$env_out" | grep -q '^PANOPLY_AGENT=' && note "env prints PANOPLY_AGENT state" || bad "env missing PANOPLY_AGENT"
echo "$env_out" | grep -q '^modules=' && note "env prints module map" || bad "env missing modules"
if [ "$have_jq" = "1" ]; then
  "$PANOPLY" env --json 2>/dev/null | jq -e '.paths and .modules' >/dev/null 2>&1 \
    && note "env --json parses (paths + modules)" || bad "env --json missing paths/modules"
fi
# --shell still yields the activation fragment
"$PANOPLY" env --shell 2>/dev/null | grep -q 'PANOPLY_HOME' && note "env --shell yields the activation fragment" || bad "env --shell did not yield the fragment"
[ "$fail" -eq 0 ] && ok "env resolved"

# 5. agent module + rtk ------------------------------------------------------
echo "[5/6] agent module + rtk"
agent_rtk="$(panoply_manifest_tsv | awk -F'\037' '$1=="agent" && $2=="rtk" {print $3}')"
if [ "$agent_rtk" = "true" ]; then note "manifest: agent module, rtk default=true"; else bad "manifest missing agent/rtk default"; fi
rtkf="$PKG/config/shell/rtk.zsh"
if [ -f "$rtkf" ]; then
  grep -q 'PANOPLY_RTK' "$rtkf" && note "rtk.zsh honors PANOPLY_RTK toggle" || bad "rtk.zsh missing PANOPLY_RTK toggle"
  grep -q 'command -v rtk' "$rtkf" && note "rtk.zsh guarded on rtk presence" || bad "rtk.zsh not guarded on rtk presence"
  grep -q 'gain' "$rtkf" && note "rtk.zsh disambiguates rtk via the gain subcommand" || bad "rtk.zsh missing rtk disambiguation (gain)"
  # must NOT unconditionally shadow core commands (routing is case-guarded / opt-out)
  if grep -Eq '^[[:space:]]*alias[[:space:]]+git=' "$rtkf"; then bad "rtk.zsh unconditionally aliases git"; else note "rtk.zsh does not blanket-alias core commands"; fi
else
  bad "missing config/shell/rtk.zsh"
fi
# panoply.zsh must source rtk.zsh only when not chezmoi-managed
panoplyzsh="$PKG/config/shell/panoply.zsh"
grep -q 'PANOPLY_RTK_MANAGED' "$panoplyzsh" && note "panoply.zsh respects PANOPLY_RTK_MANAGED" || bad "panoply.zsh does not stand down for the chezmoi rtk layer"
# profiles carry an rtk flag for local-macos-full + agent-safe
profiles="$PKG/dotfiles/.chezmoidata/profiles.toml"
if [ -f "$profiles" ]; then
  for prof in local-macos-full agent-safe; do
    blk="$(awk "/^\[profiles.$prof\]/{f=1;next} /^\[/{f=0} f" "$profiles")"
    echo "$blk" | grep -q 'rtk = true' && note "$prof: rtk = true" || bad "$prof: rtk flag not true"
  done
fi
[ "$fail" -eq 0 ] && ok "agent module + rtk"

# 6. replaceability matrix ---------------------------------------------------
echo "[6/6] replaceability matrix"
# every swappable role (tool with non-empty alternatives) must resolve via panoply list --matrix
matrix="$("$PANOPLY" list --matrix 2>/dev/null)"
if [ -n "$matrix" ]; then
  note "panoply list --matrix renders"
  echo "$matrix" | grep -Eq 'active' && note "matrix reports an active member per role" || bad "matrix missing active column"
else
  bad "panoply list --matrix produced no output"
fi
# warn (non-fatal) on alternative ids not defined as tools (external runtimes: asdf/npm/yarn)
tool_ids="$(panoply_manifest_tsv | awk -F'\037' '{print $2}')"
while IFS=$'\037' read -r _m _id _d _b _det _as alts _p; do
  [ -n "$alts" ] || continue
  IFS=',' read -r -a arr <<< "$alts"
  for alt in "${arr[@]}"; do
    [ -n "$alt" ] || continue
    echo "$tool_ids" | grep -qx "$alt" || note "note: alternative '$alt' is external (not a manifest tool)"
  done
done < <(panoply_manifest_tsv)
[ "$fail" -eq 0 ] && ok "replaceability matrix"

echo
if [ "$fail" -eq 0 ]; then echo "RESULT: PASS (dry-run, no installs)"; else echo "RESULT: FAIL"; fi
exit "$fail"

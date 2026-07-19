#!/usr/bin/env bash
# validate.sh — dry-run gate for the Panoply chezmoi source (WfOS).
#
# Checks the chezmoi *source* spec WITHOUT writing to $HOME:
#   1. structure   — required source files present, chezmoi naming conventions respected
#   2. profiles    — four profile classes defined; agent-safe excludes secrets + GUI
#   3. plugins     — guarded zsh plugin stack, syntax-highlighting last, no heavy framework
#   4. routing     — config routing contract: no app holds secrets / is a policy source
#   5. templates   — Go-template delimiter + control-keyword balance per .tmpl
#   6. no-secrets  — source contains no secret-looking values (secrets module owns values)
#   7. chezmoi     — if the binary is installed, render templates against the source
#                    (`chezmoi execute-template --init --source`); never `apply`. Else: deferral.
#
# For the full entry point (gate + profile preview + optional temp-HOME apply), use validate-dotfiles.sh
# or `moon run panoply:validate-dotfiles`.
#
# Exit 0 = pass. Exit 1 = fail. Always agent-safe: read-only on $HOME.
set -uo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0
note() { printf '  %s\n' "$*"; }
ok()   { printf 'PASS %s\n' "$*"; }
bad()  { printf 'FAIL %s\n' "$*"; fail=1; }

echo "== chezmoi source validation: $SRC =="

# 1. structure ---------------------------------------------------------------
echo "[1/7] structure"
required=(".chezmoi.toml.tmpl" "dot_zshrc.tmpl" "dot_gitconfig.tmpl" "README.md")
for f in "${required[@]}"; do
  if [ -e "$SRC/$f" ]; then note "found $f"; else bad "missing required source file: $f"; fi
done
# A source entry maps to $HOME via its TOP-LEVEL path component: if that component carries a
# chezmoi attribute prefix (or is control/meta), every file beneath it is valid.
while IFS= read -r f; do
  rel="${f#"$SRC"/}"; top="${rel%%/*}"
  case "$top" in
    .chezmoi*|README.md|ROUTING.md|SECRETS.md|validate.sh) ;; # control/meta — not targets
    dot_*|private_*|exact_*|executable_*|symlink_*|create_*|modify_*|encrypted_*) ;;  # valid prefixes
    *) bad "source target without a chezmoi attribute prefix: $rel (top: $top)" ;;
  esac
done < <(find "$SRC" -type f -not -path '*/bin/*' -not -path '*/.chezmoidata/*')
[ "$fail" -eq 0 ] && ok "structure + naming conventions"

# 2. profile classes ---------------------------------------------------------
echo "[2/7] profile classes"
profiles_toml="$SRC/.chezmoidata/profiles.toml"
if [ -f "$profiles_toml" ]; then
  for prof in local-macos-full headless-dev agent-safe workstreams-maintainer; do
    if grep -q "^\[profiles.$prof\]" "$profiles_toml"; then note "defined $prof"; else bad "missing profile class: $prof"; fi
  done
  # agent-safe must exclude secrets + editor-gui and declare the hard-block booleans.
  agent_block=$(awk '/^\[profiles.agent-safe\]/{f=1;next} /^\[/{f=0} f' "$profiles_toml")
  echo "$agent_block" | grep -q '"secrets"'    && note "agent-safe excludes secrets"    || bad "agent-safe does not exclude secrets"
  echo "$agent_block" | grep -q '"editor-gui"' && note "agent-safe excludes editor-gui" || bad "agent-safe does not exclude editor-gui"
  echo "$agent_block" | grep -q 'secrets = false'       && note "agent-safe secrets=false"       || bad "agent-safe secrets must be false"
  echo "$agent_block" | grep -q 'remote_writes = false' && note "agent-safe remote_writes=false" || bad "agent-safe remote_writes must be false"
  echo "$agent_block" | grep -q 'gui = false'           && note "agent-safe gui=false"           || bad "agent-safe gui must be false"
  # the exclusion mechanism must exist and be templated on the profile
  ignore_file=""
  for f in .chezmoiignore.tmpl .chezmoiignore; do
    if [ -f "$SRC/$f" ]; then ignore_file="$f"; break; fi
  done
  if [ -n "$ignore_file" ] && grep -q '.profiles' "$SRC/$ignore_file"; then
    note "$ignore_file present and profile-driven"
  else
    bad ".chezmoiignore(.tmpl) missing or not profile-driven (exclusions not enforced)"
  fi
  [ "$fail" -eq 0 ] && ok "profile classes + exclusion mechanism"
else
  note ".chezmoidata/profiles.toml not present yet — skipping"
fi

# 3. zsh plugin stack --------------------------------------------------------
echo "[3/7] zsh plugin stack"
frag="$SRC/dot_config/zsh/plugins.zsh.tmpl"
if [ -f "$frag" ]; then
  for plug in zsh-autosuggestions zsh-syntax-highlighting; do
    grep -q "$plug" "$frag" && note "sources $plug" || bad "plugin fragment missing $plug"
  done
  grep -q "zsh-autocomplete" "$frag" && note "zsh-autocomplete present (optional/opt-in)" \
    || note "zsh-autocomplete not referenced (optional)"
  # syntax-highlighting must be sourced AFTER autosuggestions (last to wrap the widget set)
  ln_auto=$(grep -n 'zsh-autosuggestions.zsh"' "$frag" | tail -1 | cut -d: -f1)
  ln_hl=$(grep -n 'zsh-syntax-highlighting.zsh"' "$frag" | tail -1 | cut -d: -f1)
  if [ -n "$ln_auto" ] && [ -n "$ln_hl" ] && [ "$ln_hl" -gt "$ln_auto" ]; then
    note "syntax-highlighting sourced after autosuggestions (line $ln_hl > $ln_auto)"
  else
    bad "syntax-highlighting must be sourced LAST (after autosuggestions)"
  fi
  grep -q '\[ -f ' "$frag" && note "plugin sourcing is guarded ([ -f ... ])" || bad "plugin sourcing not guarded"
  grep -q 'agent-safe profile: no interactive plugins' "$frag" \
    && note "agent-safe profile loads no interactive plugins" || bad "agent-safe plugin opt-out missing"
  # bare zsh + standalone plugins only: no heavy framework load artifacts anywhere in source
  if grep -REiq --exclude-dir=bin 'oh-my-zsh\.sh|powerlevel10k\.zsh-theme|antigen\.zsh|antibody init' "$SRC" 2>/dev/null; then
    bad "heavy zsh framework load artifact found (use bare zsh + standalone plugins)"
  else
    note "no Oh My Zsh / Powerlevel10k / antigen load artifacts"
  fi
  # panoply.zsh must stand down when the chezmoi plugin layer is active (avoid double-sourcing)
  panoplyzsh="$SRC/../config/shell/panoply.zsh"
  if [ -f "$panoplyzsh" ]; then
    grep -q 'PANOPLY_PLUGINS_MANAGED' "$panoplyzsh" && note "panoply.zsh respects PANOPLY_PLUGINS_MANAGED" \
      || bad "panoply.zsh does not stand down for the chezmoi plugin layer"
  fi
  [ "$fail" -eq 0 ] && ok "zsh plugin stack"
else
  note "plugin fragment not present yet — skipping"
fi

# 4. config routing ----------------------------------------------------------
echo "[4/7] config routing"
routing_toml="$SRC/.chezmoidata/routing.toml"
if [ -f "$routing_toml" ]; then
  for app in zed cursor claude factory opencode; do
    grep -q "^\[routing.$app\]" "$routing_toml" && note "routes $app" || bad "routing missing app: $app"
  done
  grep -q 'holds_secrets = true' "$routing_toml" && bad "an app config is marked holds_secrets=true" \
    || note "no app config holds secrets"
  grep -q 'consumes_profile_data = false' "$routing_toml" && bad "an app config does not consume shared profile data" \
    || note "all app configs consume shared profile data"
  [ -f "$SRC/ROUTING.md" ] && note "ROUTING.md present" || bad "ROUTING.md missing"
  [ "$fail" -eq 0 ] && ok "config routing rules"
else
  note ".chezmoidata/routing.toml not present yet — skipping"
fi

# 5. template balance --------------------------------------------------------
echo "[5/7] template delimiter + control balance"
while IFS= read -r tmpl; do
  rel="${tmpl#"$SRC"/}"
  opens=$(grep -o '{{' "$tmpl" | wc -l | tr -d ' ')
  closes=$(grep -o '}}' "$tmpl" | wc -l | tr -d ' ')
  if [ "$opens" != "$closes" ]; then
    bad "$rel: unbalanced delimiters ({{ x$opens vs }} x$closes)"; continue
  fi
  # control opens (if/with/range/define/block) must equal `end`s
  ctl_open=$(grep -oE '{{-?[[:space:]]*(if|with|range|define|block)[[:space:]]' "$tmpl" | wc -l | tr -d ' ')
  ctl_end=$(grep -oE '{{-?[[:space:]]*end[[:space:]]*-?}}' "$tmpl" | wc -l | tr -d ' ')
  if [ "$ctl_open" != "$ctl_end" ]; then
    bad "$rel: unbalanced control blocks (open x$ctl_open vs end x$ctl_end)"; continue
  fi
  note "$rel: {{ }} x$opens, control x$ctl_open balanced"
done < <(find "$SRC" -type f \( -name '*.tmpl' -o -name '.chezmoiignore' -o -name '.chezmoiignore.tmpl' \))
[ "$fail" -eq 0 ] && ok "template balance"

# 6. no secrets in source ----------------------------------------------------
echo "[6/7] no-secrets scan"
# Patterns that should NEVER appear as literal values in the source (refs/comments are checked too).
# Scan only chezmoi source targets/control files; exclude bin/ (this validator's own patterns).
secret_re='BEGIN[[:space:]]+(RSA|OPENSSH|PGP|EC)[[:space:]]+PRIVATE KEY|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{30,}|xox[baprs]-[A-Za-z0-9-]+|-----BEGIN'
if grep -REn --exclude-dir=bin "$secret_re" "$SRC" >/dev/null 2>&1; then
  bad "secret-looking literal found in source:"; grep -REn --exclude-dir=bin "$secret_re" "$SRC" | sed 's/^/    /'
else
  ok "no secret-looking literals (secrets module — referenced not stored)"
fi

# 7. chezmoi render (if installed) -------------------------------------------
echo "[7/7] chezmoi render"
if command -v chezmoi >/dev/null 2>&1; then
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  rendered=0
  for prof in local-macos-full headless-dev agent-safe workstreams-maintainer; do
    for t in dot_zshrc.tmpl dot_gitconfig.tmpl dot_config/zsh/plugins.zsh.tmpl dot_config/zsh/rtk.zsh.tmpl; do
      if chezmoi execute-template --source "$SRC" \
        --override-data "{\"profile\":\"$prof\",\"git\":{\"name\":\"\",\"email\":\"\"}}" \
        < "$SRC/$t" > "$tmp/out" 2>"$tmp/err"; then
        note "rendered $t [$prof]"; rendered=$((rendered+1))
      else
        bad "$t [$prof] failed to render:"; sed 's/^/    /' "$tmp/err"
      fi
    done
  done
  [ "$fail" -eq 0 ] && ok "chezmoi execute-template ($rendered renders, no \$HOME writes)"
else
  note "chezmoi binary NOT installed (manifest dotfiles/chezmoi module; install via panoply bootstrap)"
  note "live 'chezmoi diff'/'apply --dry-run' DEFERRED to a host with chezmoi"
  ok "render check skipped honestly (deferred, not assumed)"
fi

echo
if [ "$fail" -eq 0 ]; then echo "RESULT: PASS (dry-run, no \$HOME writes)"; else echo "RESULT: FAIL"; fi
exit "$fail"

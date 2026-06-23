#!/usr/bin/env bash
# validate-dotfiles.sh — dotfiles validation entry point (moon: dust:validate-dotfiles).
#
# Runs validate.sh (7-check gate), then (if chezmoi is installed) previews per-profile
# ignores and rendered templates. Never writes to your real $HOME.
#
#   bin/validate-dotfiles.sh           gate + preview
#   bin/validate-dotfiles.sh --apply   also chezmoi apply into a temp HOME (smoke test)
#
# Exit 0 on success. Exit 1 on validate failure or render errors.
set -euo pipefail

BIN="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$BIN/.." && pwd)"
DUST_HOME="$(cd "$SRC/.." && pwd)"
PROFILES=(local-macos-full headless-dev agent-safe workstreams-maintainer)
TEMPLATES=(dot_zshrc.tmpl dot_gitconfig.tmpl dot_config/zsh/plugins.zsh.tmpl)
APPLY_ISOLATED=false

usage() {
  sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY_ISOLATED=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 1 ;;
  esac
done

step=0
total=3
if [ "$APPLY_ISOLATED" = true ]; then total=4; fi

step=$((step + 1))
printf '== [%d/%d] validate ==\n' "$step" "$total"
"$BIN/validate.sh"

if ! command -v chezmoi >/dev/null 2>&1; then
  printf '\nchezmoi not installed — preview steps skipped.\n'
  printf 'Install: brew install chezmoi  (or dust bootstrap / dotfiles module)\n'
  exit 0
fi

step=$((step + 1))
printf '\n== [%d/%d] per-profile ignore preview (.chezmoiignore.tmpl) ==\n' "$step" "$total"
for prof in "${PROFILES[@]}"; do
  printf '\n--- %s ---\n' "$prof"
  ignored="$(WFOS_PROFILE="$prof" chezmoi execute-template --init --source "$SRC" \
    < "$SRC/.chezmoiignore.tmpl" 2>/dev/null | sed '/^[[:space:]]*$/d' || true)"
  if [ -n "$ignored" ]; then printf '%s\n' "$ignored"; else printf '(none)\n'; fi
done

step=$((step + 1))
printf '\n== [%d/%d] template render preview (all profiles) ==\n' "$step" "$total"
render_fail=0
for prof in "${PROFILES[@]}"; do
  for t in "${TEMPLATES[@]}"; do
    if ! WFOS_PROFILE="$prof" chezmoi execute-template --init --source "$SRC" \
      < "$SRC/$t" >/dev/null 2>&1; then
      printf 'FAIL render %s [%s]\n' "$t" "$prof" >&2
      render_fail=1
    else
      printf '  ok %s [%s]\n' "$t" "$prof"
    fi
  done
done
if [ "$render_fail" -ne 0 ]; then exit 1; fi

if [ "$APPLY_ISOLATED" = true ]; then
  step=$((step + 1))
  printf '\n== [%d/%d] isolated apply (temp HOME, profile=agent-safe) ==\n' "$step" "$total"
  test_home="$(mktemp -d)"
  # ponytail: temp dir only; trap cleans on exit or error
  trap 'rm -rf "$test_home"' EXIT
  HOME="$test_home" WFOS_PROFILE=agent-safe DUST_HOME="$DUST_HOME" \
    chezmoi apply --source "$SRC" --verbose
  printf '\n--- %s/.zshrc (head) ---\n' "$test_home"
  head -n 15 "$test_home/.zshrc"
  printf '\n--- %s/.gitconfig ---\n' "$test_home"
  cat "$test_home/.gitconfig"
  if ! grep -q 'WFOS_AGENT_SAFE=1' "$test_home/.zshrc"; then
    printf '\nFAIL: agent-safe .zshrc missing WFOS_AGENT_SAFE=1\n' >&2
    exit 1
  fi
  if ! grep -q 'default = nothing' "$test_home/.gitconfig"; then
    printf '\nFAIL: agent-safe .gitconfig missing push.default = nothing\n' >&2
    exit 1
  fi
  if [ -f "$test_home/.config/zsh/plugins.zsh" ] && \
     grep -q 'zsh-autosuggestions' "$test_home/.config/zsh/plugins.zsh" 2>/dev/null; then
    printf '\nFAIL: agent-safe plugins.zsh must not source interactive plugins\n' >&2
    exit 1
  fi
  printf '\nok: agent-safe smoke checks passed\n'
fi

printf '\nRESULT: dotfiles validation complete (real $HOME untouched)\n'

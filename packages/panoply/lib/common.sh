#!/usr/bin/env bash
# Panoply shared helpers: paths, logging, agent rails. Sourced by all bin/ entrypoints.

# Resolve a path, following symlinks, without requiring GNU readlink -f.
_panoply_realpath() {
  local target="$1" dir
  while [ -L "$target" ]; do
    local link
    link="$(readlink "$target")"
    case "$link" in
      /*) target="$link" ;;
      *) target="$(cd "$(dirname "$target")" && pwd)/$link" ;;
    esac
  done
  dir="$(cd "$(dirname "$target")" && pwd)"
  printf '%s/%s\n' "$dir" "$(basename "$target")"
}

# PANOPLY_LIB = this file's dir; PANOPLY_PKG = the panoply package root.
PANOPLY_LIB="$(cd "$(dirname "$(_panoply_realpath "${BASH_SOURCE[0]}")")" && pwd)"
PANOPLY_PKG="$(cd "$PANOPLY_LIB/.." && pwd)"
PANOPLY_MANIFEST="$PANOPLY_PKG/manifest/panoply.tools.toml"
PANOPLY_CONFIG="$PANOPLY_PKG/config"
PANOPLY_BIN="$PANOPLY_PKG/bin"
# wfos workspace root (…/workspaces/wfos) and ontarch package.
WFOS_ROOT="$(cd "$PANOPLY_PKG/../.." && pwd)"
ONTARCH_PKG="$WFOS_ROOT/packages/ontarch"
ONTARCH_REGISTRY="$ONTARCH_PKG/registry"

export PANOPLY_LIB PANOPLY_PKG PANOPLY_MANIFEST PANOPLY_CONFIG PANOPLY_BIN WFOS_ROOT ONTARCH_PKG ONTARCH_REGISTRY

# ── logging ──────────────────────────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  _C_RESET=$'\033[0m'; _C_DIM=$'\033[2m'; _C_BOLD=$'\033[1m'
  _C_GREEN=$'\033[32m'; _C_YELLOW=$'\033[33m'; _C_RED=$'\033[31m'; _C_BLUE=$'\033[34m'
else
  _C_RESET=''; _C_DIM=''; _C_BOLD=''; _C_GREEN=''; _C_YELLOW=''; _C_RED=''; _C_BLUE=''
fi

panoply_info()  { printf '%s\n' "${_C_BLUE}::${_C_RESET} $*"; }
panoply_ok()    { printf '%s\n' "${_C_GREEN}ok${_C_RESET} $*"; }
panoply_warn()  { printf '%s\n' "${_C_YELLOW}!!${_C_RESET} $*" >&2; }
panoply_err()   { printf '%s\n' "${_C_RED}xx${_C_RESET} $*" >&2; }
panoply_die()   { panoply_err "$*"; exit 1; }

# ── agent rails ──────────────────────────────────────────────────────────────
# Mutating commands must call panoply_require_human. In PANOPLY_AGENT=1 mode they are blocked.
panoply_is_agent() { [ "${PANOPLY_AGENT:-0}" = "1" ]; }

panoply_require_human() {
  if panoply_is_agent; then
    panoply_err "blocked: '${1:-this command}' is mutating and not permitted in agent mode (PANOPLY_AGENT=1)."
    panoply_err "see ontarch/policies/panoply.agent.policy.toml"
    exit 13
  fi
}

# Secret-read hard block (no_secret_read gate). Any path that would invoke a secrets-vault
# tool (pass/age/sops) to resolve a value must call this first. In PANOPLY_AGENT=1 mode it exits
# non-zero (13) so secret material can never enter agent context.
panoply_require_secret_access() {
  if panoply_is_agent; then
    panoply_err "blocked: secret read via '${1:-pass/age/sops}' is not permitted in agent mode (PANOPLY_AGENT=1)."
    panoply_err "policy no_secret_read=true — see ontarch/policies/panoply.agent.policy.toml [secrets]"
    exit 13
  fi
}

# ── tool detection ───────────────────────────────────────────────────────────
panoply_has() { command -v "$1" >/dev/null 2>&1; }

# Resolve a manifest `detect` value to an installed? check. Three forms:
#   "name"        -> command on PATH         (command -v)
#   "/abs/path"   -> absolute file/dir exists ([ -e ])
#   "rel/path"    -> file/dir under the Homebrew prefix exists (sourced plugins)
# The relative form lets non-binary tools (e.g. sourced zsh plugins under
# share/) report honestly in the registry.
panoply_detect() {
  local d="$1"
  case "$d" in
    /*) [ -e "$d" ] ;;
    */*) [ -e "${HOMEBREW_PREFIX:-$(brew --prefix 2>/dev/null)}/$d" ] ;;
    *) command -v "$d" >/dev/null 2>&1 ;;
  esac
}

panoply_version() {
  local cmd="$1"
  panoply_has "$cmd" || { printf '%s' "-"; return 1; }
  local v
  v="$("$cmd" --version 2>/dev/null | head -1)" || v=""
  [ -n "$v" ] || v="$("$cmd" -V 2>/dev/null | head -1)" || v=""
  [ -n "$v" ] || v="installed"
  printf '%s' "$v"
}

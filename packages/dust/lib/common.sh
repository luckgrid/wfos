#!/usr/bin/env bash
# Dust shared helpers: paths, logging, agent rails. Sourced by all bin/ entrypoints.

# Resolve a path, following symlinks, without requiring GNU readlink -f.
_dust_realpath() {
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

# DUST_LIB = this file's dir; DUST_PKG = the dust package root.
DUST_LIB="$(cd "$(dirname "$(_dust_realpath "${BASH_SOURCE[0]}")")" && pwd)"
DUST_PKG="$(cd "$DUST_LIB/.." && pwd)"
DUST_MANIFEST="$DUST_PKG/manifest/dust.tools.toml"
DUST_CONFIG="$DUST_PKG/config"
DUST_BIN="$DUST_PKG/bin"
# wfos workspace root (…/workspaces/wfos) and archon package.
WFOS_ROOT="$(cd "$DUST_PKG/../.." && pwd)"
ARCHON_PKG="$WFOS_ROOT/packages/archon"
ARCHON_REGISTRY="$ARCHON_PKG/registry"

export DUST_LIB DUST_PKG DUST_MANIFEST DUST_CONFIG DUST_BIN WFOS_ROOT ARCHON_PKG ARCHON_REGISTRY

# ── logging ──────────────────────────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  _C_RESET=$'\033[0m'; _C_DIM=$'\033[2m'; _C_BOLD=$'\033[1m'
  _C_GREEN=$'\033[32m'; _C_YELLOW=$'\033[33m'; _C_RED=$'\033[31m'; _C_BLUE=$'\033[34m'
else
  _C_RESET=''; _C_DIM=''; _C_BOLD=''; _C_GREEN=''; _C_YELLOW=''; _C_RED=''; _C_BLUE=''
fi

dust_info()  { printf '%s\n' "${_C_BLUE}::${_C_RESET} $*"; }
dust_ok()    { printf '%s\n' "${_C_GREEN}ok${_C_RESET} $*"; }
dust_warn()  { printf '%s\n' "${_C_YELLOW}!!${_C_RESET} $*" >&2; }
dust_err()   { printf '%s\n' "${_C_RED}xx${_C_RESET} $*" >&2; }
dust_die()   { dust_err "$*"; exit 1; }

# ── agent rails ──────────────────────────────────────────────────────────────
# Mutating commands must call dust_require_human. In DUST_AGENT=1 mode they are blocked.
dust_is_agent() { [ "${DUST_AGENT:-0}" = "1" ]; }

dust_require_human() {
  if dust_is_agent; then
    dust_err "blocked: '${1:-this command}' is mutating and not permitted in agent mode (DUST_AGENT=1)."
    dust_err "see archon/policies/dust.agent.policy.toml"
    exit 13
  fi
}

# Secret-read hard block (no_secret_read gate). Any path that would invoke a secrets-vault
# tool (pass/age/sops) to resolve a value must call this first. In DUST_AGENT=1 mode it exits
# non-zero (13) so secret material can never enter agent context.
dust_require_secret_access() {
  if dust_is_agent; then
    dust_err "blocked: secret read via '${1:-pass/age/sops}' is not permitted in agent mode (DUST_AGENT=1)."
    dust_err "policy no_secret_read=true — see archon/policies/dust.agent.policy.toml [secrets]"
    exit 13
  fi
}

# ── tool detection ───────────────────────────────────────────────────────────
dust_has() { command -v "$1" >/dev/null 2>&1; }

# Resolve a manifest `detect` value to an installed? check. Three forms:
#   "name"        -> command on PATH         (command -v)
#   "/abs/path"   -> absolute file/dir exists ([ -e ])
#   "rel/path"    -> file/dir under the Homebrew prefix exists (sourced plugins)
# The relative form lets non-binary tools (e.g. sourced zsh plugins under
# share/) report honestly in the registry.
dust_detect() {
  local d="$1"
  case "$d" in
    /*) [ -e "$d" ] ;;
    */*) [ -e "${HOMEBREW_PREFIX:-$(brew --prefix 2>/dev/null)}/$d" ] ;;
    *) command -v "$d" >/dev/null 2>&1 ;;
  esac
}

dust_version() {
  local cmd="$1"
  dust_has "$cmd" || { printf '%s' "-"; return 1; }
  local v
  v="$("$cmd" --version 2>/dev/null | head -1)" || v=""
  [ -n "$v" ] || v="$("$cmd" -V 2>/dev/null | head -1)" || v=""
  [ -n "$v" ] || v="installed"
  printf '%s' "$v"
}

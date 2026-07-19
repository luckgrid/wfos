#!/usr/bin/env bash
# Ontarch shared helpers: paths, logging. Sourced by bin/ entrypoints.
# Ontarch is data and contracts; these helpers back the build-time metadata tasks
# (sync/validate) that generate and check the registry. They never install, read
# secrets, or edit dotfiles.

# Resolve a path, following symlinks, without requiring GNU readlink -f.
_ontarch_realpath() {
  local target="$1" dir link
  while [ -L "$target" ]; do
    link="$(readlink "$target")"
    case "$link" in
      /*) target="$link" ;;
      *) target="$(cd "$(dirname "$target")" && pwd)/$link" ;;
    esac
  done
  dir="$(cd "$(dirname "$target")" && pwd)"
  printf '%s/%s\n' "$dir" "$(basename "$target")"
}

# ONTARCH_LIB = this file's dir; ONTARCH_PKG = the ontarch package root.
ONTARCH_LIB="$(cd "$(dirname "$(_ontarch_realpath "${BASH_SOURCE[0]}")")" && pwd)"
ONTARCH_PKG="$(cd "$ONTARCH_LIB/.." && pwd)"
ONTARCH_DESCRIPTORS="$ONTARCH_PKG/descriptors"
ONTARCH_SCHEMAS="$ONTARCH_PKG/schemas"
ONTARCH_POLICIES="$ONTARCH_PKG/policies"
ONTARCH_GRAPHS="$ONTARCH_PKG/graphs"
ONTARCH_REGISTRY="$ONTARCH_PKG/registry"
# wfos workspace root (…/workspaces/wfos), the workspaces dir, and Workstreams root.
# Layout: <WS_ROOT>/Build/src/workspaces/wfos — Workstreams is four levels above wfos.
WFOS_ROOT="$(cd "$ONTARCH_PKG/../.." && pwd)"
WORKSPACES_DIR="$(cd "$WFOS_ROOT/.." && pwd)"
WS_ROOT="$(cd "$WFOS_ROOT/../../../.." && pwd)"
AGENTS_HOME="$WS_ROOT/.agents"

export ONTARCH_LIB ONTARCH_PKG ONTARCH_DESCRIPTORS ONTARCH_SCHEMAS ONTARCH_POLICIES \
  ONTARCH_GRAPHS ONTARCH_REGISTRY WFOS_ROOT WORKSPACES_DIR WS_ROOT AGENTS_HOME

# ── logging ──────────────────────────────────────────────────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  _C_RESET=$'\033[0m'; _C_DIM=$'\033[2m'; _C_BOLD=$'\033[1m'
  _C_GREEN=$'\033[32m'; _C_YELLOW=$'\033[33m'; _C_RED=$'\033[31m'; _C_BLUE=$'\033[34m'
else
  _C_RESET=''; _C_DIM=''; _C_BOLD=''; _C_GREEN=''; _C_YELLOW=''; _C_RED=''; _C_BLUE=''
fi

ontarch_info() { printf '%s\n' "${_C_BLUE}::${_C_RESET} $*"; }
ontarch_ok()   { printf '%s\n' "${_C_GREEN}ok${_C_RESET} $*"; }
ontarch_warn() { printf '%s\n' "${_C_YELLOW}!!${_C_RESET} $*" >&2; }
ontarch_err()  { printf '%s\n' "${_C_RED}xx${_C_RESET} $*" >&2; }
ontarch_die()  { ontarch_err "$*"; exit 1; }

# Require jq — the sanctioned, agent-safe query tool Ontarch builds on.
ontarch_require_jq() {
  command -v jq >/dev/null 2>&1 || ontarch_die "jq not found (Panoply 'nav' module) — required for ontarch tasks"
}

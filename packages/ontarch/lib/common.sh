#!/usr/bin/env bash
# Ontarch shared helpers: paths, logging. Sourced by bin/ entrypoints.
# Ontarch is data and contracts; these helpers back the build-time metadata tasks
# (sync/validate/agents-init) that generate and check the registry. They never install,
# read secrets, or edit host dotfiles outside the agents navigation layer.

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
ONTARCH_AGENTS_PATTERN="$ONTARCH_PKG/patterns/agents"

# wfos workspace root (…/workspaces/wfos or standalone checkout root).
WFOS_ROOT="$(cd "$ONTARCH_PKG/../.." && pwd)"
WORKSPACES_DIR="$(cd "$WFOS_ROOT/.." && pwd)"

# Walk up from $1 looking for a workspace-root sentinel.
# Prefer an existing .agents/; else use README.md + Build/src/workspaces for the applied
# Workstreams layout. README is the universal worker entrypoint and namespace marker.
# Never returns "/" — fail closed for standalone/mis-laid-out checkouts.
_ontarch_find_ws_root() {
  local start="$1" dir parent
  dir="$start"
  while [ -n "$dir" ] && [ "$dir" != "/" ]; do
    if [ -d "$dir/.agents" ]; then
      printf '%s\n' "$dir"
      return 0
    fi
    if [ -f "$dir/README.md" ] && [ -d "$dir/Build/src/workspaces" ]; then
      printf '%s\n' "$dir"
      return 0
    fi
    parent="$(dirname "$dir")"
    [ "$parent" = "$dir" ] && break
    dir="$parent"
  done
  return 1
}

# Discover WS_ROOT and AGENTS_HOME.
# Precedence: AGENTS_HOME env → WS_ROOT env → sentinel walk from WFOS_ROOT →
# standalone fallback ($WFOS_ROOT/.agents). Never claim filesystem root.
_ontarch_discover_agents_home() {
  local found=""

  if [ -n "${AGENTS_HOME:-}" ]; then
    case "$AGENTS_HOME" in
      /*) ;;
      *) AGENTS_HOME="$(cd "$AGENTS_HOME" 2>/dev/null && pwd)" || AGENTS_HOME="" ;;
    esac
    if [ -n "$AGENTS_HOME" ]; then
      WS_ROOT="$(dirname "$AGENTS_HOME")"
      return 0
    fi
  fi

  if [ -n "${WS_ROOT:-}" ]; then
    case "$WS_ROOT" in
      /*) ;;
      *) WS_ROOT="$(cd "$WS_ROOT" 2>/dev/null && pwd)" || WS_ROOT="" ;;
    esac
    if [ -n "$WS_ROOT" ] && [ "$WS_ROOT" != "/" ]; then
      AGENTS_HOME="$WS_ROOT/.agents"
      return 0
    fi
  fi

  if found="$(_ontarch_find_ws_root "$WFOS_ROOT")"; then
    WS_ROOT="$found"
    AGENTS_HOME="$WS_ROOT/.agents"
    return 0
  fi

  # Standalone wfos checkout: materialize beside the workspace root.
  WS_ROOT="$WFOS_ROOT"
  AGENTS_HOME="$WFOS_ROOT/.agents"
}

_ontarch_discover_agents_home

export ONTARCH_LIB ONTARCH_PKG ONTARCH_DESCRIPTORS ONTARCH_SCHEMAS ONTARCH_POLICIES \
  ONTARCH_GRAPHS ONTARCH_REGISTRY ONTARCH_AGENTS_PATTERN WFOS_ROOT WORKSPACES_DIR \
  WS_ROOT AGENTS_HOME

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

# Read id= / version= from PATTERN.toml (flat keys only).
ontarch_pattern_field() {
  local file="$1" key="$2"
  awk -F' *= *' -v k="$key" '
    $1 == k {
      v = $2
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      gsub(/^"|"$/, "", v)
      print v
      exit
    }
  ' "$file"
}

# Resolve a kind=template body_ref: working copy first, then the Ontarch pattern seed.
# body_ref is relative to skills/ (e.g. templates/adr.md). Prints absolute path or empty.
ontarch_template_body_path() {
  local body_ref="$1" candidate
  candidate="$AGENTS_HOME/skills/$body_ref"
  if [ -f "$candidate" ]; then
    printf '%s\n' "$candidate"
    return 0
  fi
  candidate="$ONTARCH_AGENTS_PATTERN/skills/$body_ref"
  if [ -f "$candidate" ]; then
    printf '%s\n' "$candidate"
    return 0
  fi
  return 1
}

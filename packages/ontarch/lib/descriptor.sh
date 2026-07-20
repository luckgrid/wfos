#!/usr/bin/env bash
# Ontarch descriptor reader: parse the canon TOML descriptor subset into JSON.
# Sourced after common.sh (uses jq). Dependency-free TOML handling via awk, mirroring
# the Panoply manifest reader. Supports the regular subset the canon uses (§10.1):
#   top-level   key = "value" | true/false | number | ["a", "b"]
#   tables       [section] with the same key forms
# Arrays must be single-line and inline. Inline comments after a value are not supported
# (descriptors are kept clean); whole-line `#` comments are ignored.

# Emit a US-delimited (0x1f) normalized stream, one record per key:
#   scope \037 key \037 kind \037 value
# scope = "." for top-level, else the table name. kind ∈ {s,b,n,a}.
# For arrays (kind=a) the value is the raw inside-bracket text (already valid JSON
# array contents because elements are double-quoted), e.g.  "a", "b"
ontarch_descriptor_stream() {
  awk '
    BEGIN { US="\037"; scope="." }
    {
      line=$0
      sub(/^[[:space:]]+/, "", line); sub(/[[:space:]]+$/, "", line)
      if (line=="" || line ~ /^#/) next
      if (line ~ /^\[[^]]+\][[:space:]]*$/) {
        s=line; sub(/^\[/, "", s); sub(/\][[:space:]]*$/, "", s)
        scope=s; next
      }
      eq=index(line, "=")
      if (eq==0) next
      key=substr(line, 1, eq-1); val=substr(line, eq+1)
      sub(/[[:space:]]+$/, "", key); sub(/^[[:space:]]+/, "", key)
      sub(/^[[:space:]]+/, "", val); sub(/[[:space:]]+$/, "", val)
      if (val ~ /^\[/) {
        kind="a"; sub(/^\[/, "", val); sub(/\][[:space:]]*$/, "", val)
      } else if (val=="true" || val=="false") {
        kind="b"
      } else if (val ~ /^-?[0-9]+$/ || val ~ /^-?[0-9]+\.[0-9]+$/) {
        kind="n"
      } else {
        kind="s"
        if (val ~ /^".*"$/) { sub(/^"/, "", val); sub(/"$/, "", val) }
      }
      print scope US key US kind US val
    }
  ' "$1"
}

# Build a JSON object for all keys in one scope of a descriptor stream.
_ontarch_scope_object() {
  local stream="$1" scope="$2" obj='{}' sc key knd val jval
  while IFS=$'\037' read -r sc key knd val; do
    [ "$sc" = "$scope" ] || continue
    case "$knd" in
      a) jval="$(printf '[%s]' "$val" | jq -c .)"
         obj="$(jq -c --arg k "$key" --argjson v "$jval" '. + {($k): $v}' <<<"$obj")" ;;
      b|n) obj="$(jq -c --arg k "$key" --argjson v "$val" '. + {($k): $v}' <<<"$obj")" ;;
      *) obj="$(jq -c --arg k "$key" --arg v "$val" '. + {($k): $v}' <<<"$obj")" ;;
    esac
  done <<<"$stream"
  printf '%s' "$obj"
}

# Convert a descriptor TOML file to a full nested JSON object (compact).
# Flat tables (`[cli]`) become nested objects. Dotted tables (`[entrypoints.dev]`)
# nest via JSON path so structured lifecycle entrypoints survive projection.
ontarch_descriptor_json() {
  local f="$1" stream out scopes s nested
  stream="$(ontarch_descriptor_stream "$f")"
  out="$(_ontarch_scope_object "$stream" .)"
  scopes="$(printf '%s\n' "$stream" | awk -F'\037' '$1!="." {print $1}' | sort -u)"
  for s in $scopes; do
    nested="$(_ontarch_scope_object "$stream" "$s")"
    if [[ "$s" == *.* ]]; then
      out="$(jq -c --arg path "$s" --argjson v "$nested" '
        ($path | split(".")) as $parts |
        setpath($parts; ((getpath($parts) // {}) + $v))
      ' <<<"$out")"
    else
      out="$(jq -c --arg k "$s" --argjson v "$nested" '
        . + {($k): ((.[$k] // {}) + $v)}
      ' <<<"$out")"
    fi
  done
  printf '%s' "$out"
}

# Discover descriptor files. Emits `path \037 source` lines, source ∈ {central, colocated}.
# Colocated-first authoring; central (ontarch/descriptors/) is a valid override location.
# Colocated locations: each Build/src/workspaces/<ws>/ root, and wfos packages/<pkg>/ and
# apps/<app>/ directories.
ontarch_find_descriptors() {
  local f wsdir
  for f in "$ONTARCH_DESCRIPTORS"/*.descriptor.toml; do
    [ -e "$f" ] && printf '%s\037central\n' "$f"
  done
  for wsdir in "$WORKSPACES_DIR"/*/; do
    for f in "$wsdir"*.descriptor.toml; do
      [ -e "$f" ] && printf '%s\037colocated\n' "$f"
    done
  done
  for f in "$WFOS_ROOT"/packages/*/*.descriptor.toml "$WFOS_ROOT"/apps/*/*.descriptor.toml; do
    [ -e "$f" ] && printf '%s\037colocated\n' "$f"
  done
  return 0
}

# Project a full descriptor JSON into a compact registry unit record.
# Routing-relevant entrypoints (string or structured) and CLI metadata must survive
# so a ResolvedCommand can be constructed without re-reading the TOML source.
# Args: <full-json> <source: colocated|central>
ontarch_unit_record() {
  jq -c --arg source "$2" '{
    id, kind, title, status,
    domain: (.domain // .system_space // null),
    layer: (.layer // null),
    path: (.paths.root // null),
    native_manifests: (.native.manifests // []),
    entrypoints: (.entrypoints // {}),
    cli: (if .cli then {
      entry: (.cli.entry // null),
      commands: (.cli.commands // [])
    } else null end),
    provides: (.capabilities.provides // []),
    requires: (.capabilities.requires // []),
    policy: (.policy // {}),
    source: $source
  }' <<<"$1"
}

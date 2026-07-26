#!/usr/bin/env bash
# Ontarch registry emitters: generate the registry JSON from descriptors, policies, and the
# .agents/ navigation layer. Sourced after common.sh + descriptor.sh (uses jq).
# Output is compact (one record per line, like tools.json) so RTK + jq stay cheap.

_ontarch_now() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Join a compact JSON array into a single inline `a,b,c` string (empty if no elements).
_ontarch_inline() { jq -c '.[]' <<<"$1" | paste -sd, -; }

# SHA-256 hex digest of a file's raw bytes (macOS shasum or sha256sum).
_ontarch_sha256_file() {
  local f="$1"
  [ -f "$f" ] || return 1
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$f" | awk '{print $1}'
  else
    sha256sum "$f" | awk '{print $1}'
  fi
}

# Build registry_generation JSON from authored source file paths.
# Prints: { generated_at, source_fingerprints: [{path, algorithm, digest}, ...] }
ontarch_registry_generation() {
  local ts="$(_ontarch_now)"
  local fps='[]' f rel digest
  for f in "$@"; do
    [ -f "$f" ] || continue
    digest="$(_ontarch_sha256_file "$f")" || continue
    rel="${f#"$WS_ROOT"/}"
    [ "$rel" = "$f" ] && rel="$f"
    fps="$(jq -c --arg path "$rel" --arg digest "$digest" \
      '. + [{path: $path, algorithm: "sha256", digest: $digest}]' <<<"$fps")"
  done
  fps="$(jq -c 'sort_by(.path)' <<<"$fps")"
  jq -nc --arg ts "$ts" --argjson fps "$fps" \
    '{generated_at: $ts, source_fingerprints: $fps}'
}

# units.json — colocated-first discovery, central overrides colocated for a shared id.
ontarch_emit_units() {
  local units='[]' f src full id rec
  local -A from_central=()
  local -a source_files=()
  while IFS=$'\037' read -r f src; do
    [ -n "$f" ] || continue
    source_files+=("$f")
    full="$(ontarch_descriptor_json "$f")"
    id="$(jq -r '.id' <<<"$full")"
    rec="$(ontarch_unit_record "$full" "$src")"
    if [ "$src" = "central" ]; then
      from_central[$id]=1
    elif [ -n "${from_central[$id]:-}" ]; then
      continue   # central already won for this id
    fi
    units="$(jq -c --argjson r "$rec" 'map(select(.id != $r.id)) + [$r]' <<<"$units")"
  done < <(ontarch_find_descriptors)

  units="$(jq -c 'sort_by(.id)' <<<"$units")"
  local summary gen
  summary="$(jq -c '{
    total: length,
    by_kind: (group_by(.kind) | map({key: .[0].kind, value: length}) | from_entries)
  }' <<<"$units")"
  gen="$(ontarch_registry_generation "${source_files[@]}")"
  jq -n --argjson gen "$gen" --argjson summary "$summary" --argjson units "$units" '{
    generated_at: $gen.generated_at,
    registry_generation: $gen,
    summary: $summary,
    units: $units
  }'
}

# policies.json — index every policy TOML (parsed via the descriptor reader) with its source.
ontarch_emit_policies() {
  local arr='[]' f full rel
  for f in "$ONTARCH_POLICIES"/*.toml; do
    [ -e "$f" ] || continue
    full="$(ontarch_descriptor_json "$f")"
    rel="${f#"$WS_ROOT"/}"
    arr="$(jq -c --argjson p "$full" --arg src "$rel" '. + [$p + {source: $src}]' <<<"$arr")"
  done
  printf '{\n  "generated_at": "%s",\n  "policies": [%s]\n}\n' \
    "$(_ontarch_now)" "$(_ontarch_inline "$arr")"
}

# Project a full skill JSON (nested TOML tables) into a compact registry record using the
# exact field names the skills module contract declares.
# Args: <full-json>
ontarch_skill_record() {
  jq -c '{
    skill_id: .id,
    source,
    kind,
    body_ref: (.body_ref // .id),
    version: (.version // null),
    supported_agent_apps: (.supported_agent_apps // []),
    allowed_contexts: (.allowed_contexts // []),
    inputs: (.inputs // {}),
    outputs: (.outputs // {}),
    touches: (.touches // []),
    risks: (.risks // []),
    validator: (.validator // null),
    scan: {
      status: (.scan.status // "unscanned"),
      scanner: (.scan.scanner // null),
      hash: (.scan.hash // ""),
      scanned_at: (.scan.scanned_at // "")
    }
  }' <<<"$1"
}

# skills.json — curated skill records from .agents/skills/*.toml, flattened by ontarch_skill_record.
ontarch_emit_skills() {
  local arr='[]' f full rec
  if [ -d "$AGENTS_HOME/skills" ]; then
    for f in "$AGENTS_HOME"/skills/*.toml; do
      [ -e "$f" ] || continue
      full="$(ontarch_descriptor_json "$f")"
      rec="$(ontarch_skill_record "$full")"
      arr="$(jq -c --argjson s "$rec" '. + [$s]' <<<"$arr")"
    done
  fi
  arr="$(jq -c 'sort_by(.skill_id)' <<<"$arr")"
  printf '{\n  "generated_at": "%s",\n  "skills": [%s]\n}\n' \
    "$(_ontarch_now)" "$(_ontarch_inline "$arr")"
}

# scan.json — read-only polyrepo scan report over Build/src/workspaces. One report replaces N
# per-repo `git status` reads. Every field comes from read-only `git -C <dir>` plus the already
# generated units.json (kind/manifests) and profiles.json (agent scope rules). No writes, no
# remote operations. See schemas/scan.schema.json.
ontarch_emit_scan() {
  local units="$ONTARCH_REGISTRY/units.json"
  local profiles="$ONTARCH_REGISTRY/profiles.json"
  local units_json profiles_json
  units_json="$( [ -f "$units" ] && cat "$units" || echo '{"units":[]}' )"
  profiles_json="$( [ -f "$profiles" ] && cat "$profiles" || echo '{"profiles":[]}' )"

  local arr='[]' d rel git_root active def remotes porc changed wt manifests f
  for d in "$WORKSPACES_DIR"/*/; do
    d="${d%/}"
    [ -d "$d/.git" ] || continue
    rel="${d#"$WS_ROOT"/}"
    git_root="$(git -C "$d" rev-parse --show-toplevel 2>/dev/null || echo "")"
    active="$(git -C "$d" branch --show-current 2>/dev/null || echo "")"
    def="$(git -C "$d" symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null | sed 's#^origin/##' || echo "")"
    remotes="$(git -C "$d" remote 2>/dev/null | jq -R 'select(length>0)' | jq -sc .)"
    [ -n "$remotes" ] || remotes='[]'
    porc="$(git -C "$d" status --porcelain 2>/dev/null || true)"
    changed="$(printf '%s' "$porc" | grep -c . || true)"; changed="${changed//[^0-9]/}"; changed="${changed:-0}"
    wt="$(git -C "$d" worktree list --porcelain 2>/dev/null | grep -c '^worktree ' || true)"; wt="${wt//[^0-9]/}"; wt="${wt:-1}"

    # Native manifests: detect the common roots present in the workspace root.
    manifests='[]'
    for f in package.json Cargo.toml go.mod pyproject.toml moon.yml .prototools deno.json; do
      [ -f "$d/$f" ] && manifests="$(jq -c --arg m "$f" '. + [$m]' <<<"$manifests")"
    done

    arr="$(jq -c \
      --arg path "$rel" --arg git_root "$git_root" --arg active "$active" --arg def "$def" \
      --arg wsname "$(basename "$WS_ROOT")" \
      --argjson remotes "$remotes" --argjson changed "${changed:-0}" --argjson wt "${wt:-1}" \
      --argjson manifests "$manifests" \
      --argjson units "$units_json" --argjson profiles "$profiles_json" '
      ($units.units // []) as $U |
      ($profiles.profiles // []) as $P |
      ($U | map(select(.path == $path)) | .[0]) as $unit |
      . + [{
        path: $path,
        kind: ($unit.kind // "workspace"),
        git_root: $git_root,
        remote_set: $remotes,
        default_branch: (if $def == "" then null else $def end),
        active_branch: (if $active == "" then null else $active end),
        worktree_status: {
          state: (if $changed == 0 then "clean" else "dirty" end),
          changed: $changed,
          worktrees: $wt
        },
        native_manifests: (($unit.native_manifests // []) + $manifests | unique),
        lint_check_commands: (($unit.entrypoints // {}) | to_entries | map(.value | tostring)),
        agent_scope_rules: [
          $P[] | . as $pr |
          (($pr.allowed_paths // []) | map(sub("^" + $wsname + "/"; "") | sub("/\\*+$"; "")) |
            any(. as $g | ($path | startswith($g)) or ($g | startswith($path)))) as $inscope |
          (($pr.blocked_paths // []) | map(sub("^" + $wsname + "/"; "") | sub("/\\*+$"; "")) |
            any(. as $g | ($path | startswith($g)))) as $blocked |
          {profile: $pr.id, in_scope: $inscope, blocked: $blocked}
        ]
      }]' <<<"$arr")"
  done

  arr="$(jq -c 'sort_by(.path)' <<<"$arr")"
  local total clean dirty gen
  total="$(jq 'length' <<<"$arr")"
  clean="$(jq '[.[] | select(.worktree_status.state == "clean")] | length' <<<"$arr")"
  dirty="$(jq '[.[] | select(.worktree_status.state == "dirty")] | length' <<<"$arr")"
  # Fingerprint authored inputs that feed the scan report (units + profiles registries).
  gen="$(ontarch_registry_generation "$units" "$profiles")"
  jq -n --arg ts "$(_ontarch_now)" --arg root "$WORKSPACES_DIR" \
    --argjson gen "$gen" \
    --argjson total "$total" --argjson clean "$clean" --argjson dirty "$dirty" \
    --argjson ws "$arr" '{
      generated_at: $ts,
      registry_generation: $gen,
      root: $root,
      summary: {total: $total, clean: $clean, dirty: $dirty},
      workspaces: $ws
    }'
}

# local-toolkit.yml — the .agents/ navigation view of the toolkit, derived from the Panoply
# manifest + tools.json. Each tool gets one mutually-exclusive status:
#   present   = installed on this host
#   missing   = a module-default that is absent (should be installed)
#   candidate = an optional tool (default=false) not installed — available to adopt
#   deprecated= flagged for removal (none today; taxonomy slot)
ontarch_emit_local_toolkit() {
  local tools="$ONTARCH_REGISTRY/tools.json"
  [ -f "$tools" ] || { ontarch_warn "tools.json absent — run 'panoply doctor' before sync to emit local-toolkit.yml"; return 1; }
  local classified mver host cp cm cc bucket items
  mver="$(jq -r '.manifest_version' "$tools")"
  host="$(jq -r '.host' "$tools")"
  classified="$(jq -c '.tools | map({id, module, default,
    status: (if .installed then "present" elif .default then "missing" else "candidate" end)})' "$tools")"
  cp=$(jq -r '[.[]|select(.status=="present")]  | length' <<<"$classified")
  cm=$(jq -r '[.[]|select(.status=="missing")]  | length' <<<"$classified")
  cc=$(jq -r '[.[]|select(.status=="candidate")]| length' <<<"$classified")

  printf '# GENERATED by `ontarch sync` from the Panoply manifest + ontarch/registry/tools.json.\n'
  printf '# Do not hand-edit — regenerate with `moon run ontarch:sync`. Host-specific (gitignored).\n'
  printf 'generated_at: "%s"\n' "$(_ontarch_now)"
  printf 'manifest_version: "%s"\n' "$mver"
  printf 'host: "%s"\n' "$host"
  printf 'summary: { present: %s, missing: %s, candidate: %s, deprecated: 0 }\n' "$cp" "$cm" "$cc"
  for bucket in present missing candidate deprecated; do
    items="$(jq -r --arg s "$bucket" \
      '[.[]|select(.status==$s)] | sort_by(.id) | .[] | "  - { id: \(.id), module: \(.module), default: \(.default) }"' \
      <<<"$classified")"
    if [ -n "$items" ]; then printf '%s:\n%s\n' "$bucket" "$items"; else printf '%s: []\n' "$bucket"; fi
  done
}

# graph.json — the project relationship graph, derived from units.json + policies.json.
# Nodes: units (kind from descriptor), capabilities (capability:<name>), policies
# (policy:<id>), and an actor node ("agent") when a policy applies_to="agent".
# Edges: unit -provides-> capability, unit -requires-> capability,
#        unit -uses-> unit (when requires∩provides across units),
#        unit -governed-> policy (when policy.applies_to == unit.id),
#        agent -blocked-by-> policy (when policy.applies_to == "agent").
ontarch_emit_graph() {
  local units="$ONTARCH_REGISTRY/units.json"
  local policies="$ONTARCH_REGISTRY/policies.json"
  local profiles="$ONTARCH_REGISTRY/profiles.json"
  local skills="$ONTARCH_REGISTRY/skills.json"
  [ -f "$units" ]   || { ontarch_warn "units.json absent — graph requires sync to run first"; return 1; }
  [ -f "$policies" ] || { ontarch_warn "policies.json absent — graph requires sync to run first"; return 1; }
  # Defensive empty docs are fingerprinted (must exist before digest).
  local ts_empty="$(_ontarch_now)"
  [ -f "$profiles" ] || printf '{"generated_at":"%s","profiles":[]}\n' "$ts_empty" > "$profiles"
  [ -f "$skills" ]   || printf '{"generated_at":"%s","skills":[]}\n' "$ts_empty" > "$skills"

  # Exact four upstream registry docs; stable package-relative paths; sorted by path.
  local gen fps='[]' name f digest
  for name in policies.json profiles.json skills.json units.json; do
    f="$ONTARCH_REGISTRY/$name"
    [ -f "$f" ] || { ontarch_err "graph fingerprint: missing $f"; return 1; }
    digest="$(_ontarch_sha256_file "$f")" || { ontarch_err "graph fingerprint: digest failed for $f"; return 1; }
    fps="$(jq -c --arg path "registry/$name" --arg digest "$digest" \
      '. + [{path: $path, algorithm: "sha256", digest: $digest}]' <<<"$fps")"
  done
  fps="$(jq -c 'sort_by(.path)' <<<"$fps")"
  gen="$(jq -nc --arg ts "$(_ontarch_now)" --argjson fps "$fps" \
    '{generated_at: $ts, source_fingerprints: $fps}')"

  jq -n --argjson gen "$gen" \
    --slurpfile U "$units" --slurpfile P "$policies" \
    --slurpfile PR "$profiles" --slurpfile SK "$skills" '
    ($U[0].units)    as $units    |
    ($P[0].policies) as $policies |
    (($PR[0].profiles) // []) as $profiles |
    (($SK[0].skills) // []) as $skills |
    ($skills | map(.skill_id)) as $skill_ids |
    ($units | map(. as $u | (.provides // [])[] | {from: $u.id, rel: "provides", to: ("capability:" + .)}))
      as $provides_edges |
    ($units | map(. as $u | (.requires // [])[] | {from: $u.id, rel: "requires", to: ("capability:" + .)}))
      as $requires_edges |
    ([ $units[] as $u | $units[] as $v |
      select($u.id != $v.id) |
      select(
        ($u.requires // []) as $reqs | ($v.provides // []) as $provs |
        any($reqs[]; . as $r | $provs | index($r))
      ) | {from: $u.id, rel: "uses", to: $v.id} ])
      as $uses_edges |
    ($units | map(. as $u | ($policies | map(select(.applies_to == $u.id)) | .[] |
               {from: $u.id, rel: "governed-by", to: ("policy:" + .id)})))
      as $governed_edges |
    [($policies | map(select(.applies_to == "agent")) | .[] |
               {from: "agent", rel: "blocked-by", to: ("policy:" + .id)})]
      as $blocked_edges |
    ([$policies[].id] | unique) as $policy_ids |
    ($profiles | map(. as $pr | select(($pr.rails // null) != null and ($policy_ids | index($pr.rails))) |
               {from: ("profile:" + $pr.id), rel: "selects", to: ("policy:" + $pr.rails)}))
      as $selects_rails_edges |
    ($profiles | map(. as $pr | select(($pr.rails_bin // null) != null and ($policy_ids | index($pr.rails_bin))) |
               {from: ("profile:" + $pr.id), rel: "selects", to: ("policy:" + $pr.rails_bin)}))
      as $selects_bin_edges |
    ($selects_rails_edges + $selects_bin_edges) as $selects_edges |
    ($skills | map({id: ("skill:" + .skill_id), kind: "skill"})) as $skill_nodes |
    ($profiles | map(. as $pr |
      ($pr.allowed_skill_ids // [])[] |
      select(. as $sid | $skill_ids | index($sid)) |
      {from: ("profile:" + $pr.id), rel: "can-invoke", to: ("skill:" + .)}))
      as $can_invoke_edges |
    ($units | map({id: .id, kind: .kind})) as $unit_nodes |
    (($provides_edges + $requires_edges | map(.to) | unique) | map({id: ., kind: "capability"}))
      as $cap_nodes |
    ($policies | map({id: ("policy:" + .id), kind: "policy"})) as $policy_nodes |
    (if ($blocked_edges | length) > 0 then [{id: "agent", kind: "actor"}] else [] end)
      as $actor_nodes |
    ($profiles | map({id: ("profile:" + .id), kind: "profile"})) as $profile_nodes |
    {
      generated_at: $gen.generated_at,
      registry_generation: $gen,
      nodes: ($unit_nodes + $cap_nodes + $policy_nodes + $actor_nodes + $profile_nodes + $skill_nodes
              | sort_by(.kind, .id)),
      edges: ($provides_edges + $requires_edges + $uses_edges + $governed_edges + $blocked_edges
              + $selects_edges + $can_invoke_edges
              | sort_by(.from, .rel, .to))
    }
  '
}

# graph.dot — Graphviz DOT rendering, derived from graph.json (read from stdin).
ontarch_emit_graph_dot() {
  # Escape quotes/backslashes for Graphviz; callers must reject control chars first.
  # Character-wise replace avoids jq regex replacement doubling pitfalls.
  jq -r '
    def esc:
      tostring
      | [splits("")]
      | map(if . == "\\" then "\\\\" elif . == "\"" then "\\\"" else . end)
      | join("");
    "digraph ontarch {\n  rankdir=LR;\n  node [shape=box];\n",
    (.edges[] | "  \"\(.from | esc)\" -> \"\(.to | esc)\" [label=\"\(.rel | esc)\"];\n"),
    "}\n"
  '
}

# Project a full profile JSON (nested TOML tables) into a compact registry record using the
# exact field names the epic contract declares. Mirrors ontarch_unit_record.
# Args: <full-json>
ontarch_profile_record() {
  jq -c '{
    id, title, purpose,
    rails: (.rails // null),
    rails_bin: (.rails_bin // null),
    allowed_paths: (.scope.allowed_paths // []),
    blocked_paths: (.scope.blocked_paths // []),
    allowed_commands: (.commands.allowed_commands // []),
    gated_commands: (.commands.gated_commands // []),
    blocked_commands: (.commands.blocked_commands // []),
    secret_access: (.policy.secret_access // false),
    remote_write_policy: (.policy.remote_write_policy // "blocked"),
    isolation_mode: (.isolation.mode // "main"),
    isolation_jj: (.isolation.jj // "off"),
    loads_external_skills: (.skills.loads_external // false),
    allowed_skill_ids: (.skills.allowed_skill_ids // []),
    required_validators: (.validators.required_validators // []),
    output_compressor: (.output.compressor // null),
    session_log_target: (.logs.session_log_target // null),
    session_state_home: (.runtime.session_state_home // null)
  }' <<<"$1"
}

# profiles.json — populated from .agents/profiles/*.toml. Each profile is read by the
# Ontarch TOML reader and flattened by ontarch_profile_record into a compact record.
ontarch_emit_profiles() {
  local arr='[]' f full rec
  if [ -d "$AGENTS_HOME/profiles" ]; then
    for f in "$AGENTS_HOME"/profiles/*.toml; do
      [ -e "$f" ] || continue
      full="$(ontarch_descriptor_json "$f")"
      rec="$(ontarch_profile_record "$full")"
      arr="$(jq -c --argjson p "$rec" '. + [$p]' <<<"$arr")"
    done
  fi
  printf '{\n  "generated_at": "%s",\n  "profiles": [%s]\n}\n' \
    "$(_ontarch_now)" "$(_ontarch_inline "$arr")"
}

# Count files under a directory (fd preferred; find fallback).
# Args: <dir>
_ontarch_count_files() {
  local dir="$1" n
  if command -v fd >/dev/null 2>&1; then
    n="$(fd --type f --hidden --no-ignore . "$dir" 2>/dev/null | wc -l | tr -d ' ')"
  else
    # ponytail: find fallback when fd is absent; same count semantics
    n="$(find "$dir" -type f 2>/dev/null | wc -l | tr -d ' ')"
  fi
  printf '%s\n' "${n:-0}"
}

# Count manifest.json files under a directory (recursive).
# Args: <dir>
_ontarch_count_manifests() {
  local dir="$1" n
  if command -v fd >/dev/null 2>&1; then
    n="$(fd --type f --hidden --no-ignore '^manifest\.json$' "$dir" 2>/dev/null | wc -l | tr -d ' ')"
  else
    n="$(find "$dir" -type f -name 'manifest.json' 2>/dev/null | wc -l | tr -d ' ')"
  fi
  printf '%s\n' "${n:-0}"
}

# Age in whole days of oldest and newest files under <dir>. Prints "oldest newest"
# (empty strings when the tree has no files). Uses portable stat.
# Args: <dir>
_ontarch_file_age_days() {
  local dir="$1" now oldest newest mtime age
  now="$(date +%s)"
  oldest=""
  newest=""
  # Collect mtimes: prefer fd paths, fall back to find.
  local paths
  if command -v fd >/dev/null 2>&1; then
    paths="$(fd --type f --hidden --no-ignore . "$dir" 2>/dev/null || true)"
  else
    paths="$(find "$dir" -type f 2>/dev/null || true)"
  fi
  [ -n "$paths" ] || { printf ' \n'; return 0; }
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    # Prefer GNU -c %Y first: on Linux, BSD-style `stat -f` is --file-system and can
    # succeed with non-epoch output. Fall back to BSD -f %m (macOS). Digits-only guard.
    mtime="$(stat -c %Y "$f" 2>/dev/null || true)"
    case "$mtime" in
      ''|*[!0-9]*) mtime="$(stat -f %m "$f" 2>/dev/null || true)" ;;
    esac
    case "$mtime" in
      ''|*[!0-9]*) continue ;;
    esac
    age=$(( (now - mtime) / 86400 ))
    [ "$age" -lt 0 ] && age=0
    if [ -z "$oldest" ] || [ "$age" -gt "$oldest" ]; then oldest="$age"; fi
    if [ -z "$newest" ] || [ "$age" -lt "$newest" ]; then newest="$age"; fi
  done <<<"$paths"
  printf '%s %s\n' "$oldest" "$newest"
}

# Human-readable size from bytes (KiB/MiB/GiB).
# Args: <bytes>
_ontarch_human_size() {
  local b="$1"
  if [ "$b" -ge 1073741824 ]; then
    awk -v b="$b" 'BEGIN { printf "%.1fGiB", b/1073741824 }'
  elif [ "$b" -ge 1048576 ]; then
    awk -v b="$b" 'BEGIN { printf "%.1fMiB", b/1048576 }'
  elif [ "$b" -ge 1024 ]; then
    awk -v b="$b" 'BEGIN { printf "%.1fKiB", b/1024 }'
  else
    printf '%sB' "$b"
  fi
}

# bin-inventory.json — read-only inventory of Workstreams/*/bin/<workflow>/ directories.
# One report replaces N du/ls/stat explorations. Writes nothing under bin/.
ontarch_emit_bin_inventory() {
  local arr='[]' ns_bin workflow rel size_k size_bytes file_count manifest_count
  local oldest newest ages present
  for ns_bin in "$WS_ROOT"/*/bin; do
    [ -d "$ns_bin" ] || continue
    for workflow in "$ns_bin"/*/; do
      [ -d "$workflow" ] || continue
      workflow="${workflow%/}"
      # Skip hidden dirs
      case "$(basename "$workflow")" in .*) continue ;; esac
      rel="${workflow#"$WS_ROOT"/}"
      size_k="$(du -sk "$workflow" 2>/dev/null | awk '{print $1}')"
      size_k="${size_k:-0}"
      size_bytes=$(( size_k * 1024 ))
      file_count="$(_ontarch_count_files "$workflow")"
      manifest_count="$(_ontarch_count_manifests "$workflow")"
      ages="$(_ontarch_file_age_days "$workflow")"
      oldest="${ages%% *}"
      newest="${ages#* }"
      [ "$newest" = "$ages" ] && newest=""
      if [ "$manifest_count" -gt 0 ]; then present=true; else present=false; fi

      arr="$(jq -c \
        --arg path "$rel" \
        --argjson size "$size_bytes" \
        --argjson files "$file_count" \
        --argjson mc "$manifest_count" \
        --argjson present "$present" \
        --arg oldest "$oldest" \
        --arg newest "$newest" '
        . + [{
          path: $path,
          size_bytes: $size,
          file_count: $files,
          oldest_file_age_days: (if $oldest == "" then null else ($oldest | tonumber) end),
          newest_file_age_days: (if $newest == "" then null else ($newest | tonumber) end),
          manifest_present: $present,
          manifest_count: $mc
        }]' <<<"$arr")"
    done
  done

  arr="$(jq -c 'sort_by(.path)' <<<"$arr")"
  local total with_manifest
  total="$(jq 'length' <<<"$arr")"
  with_manifest="$(jq '[.[] | select(.manifest_present == true)] | length' <<<"$arr")"
  jq -n --arg ts "$(_ontarch_now)" --arg root "$WS_ROOT" \
    --argjson total "$total" --argjson with_manifest "$with_manifest" \
    --argjson entries "$arr" '{
      generated_at: $ts,
      root: $root,
      summary: { total: $total, with_manifest: $with_manifest },
      workflows: $entries
    }'
}

# Emit a run manifest (stdout). Required args via named parameters:
#   --id --workflow --source --tool --retention
# Optional: --approved-to (default null), --created-at (default now compact),
#           --tool-version, --notes
# Outputs: remaining positional args (at least one required).
ontarch_emit_manifest() {
  local id="" workflow="" source="" tool="" retention="" approved_to=""
  local created_at="" tool_version="" notes=""
  local -a outputs=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --id) id="$2"; shift 2 ;;
      --workflow) workflow="$2"; shift 2 ;;
      --source) source="$2"; shift 2 ;;
      --tool) tool="$2"; shift 2 ;;
      --retention) retention="$2"; shift 2 ;;
      --approved-to) approved_to="$2"; shift 2 ;;
      --created-at) created_at="$2"; shift 2 ;;
      --tool-version) tool_version="$2"; shift 2 ;;
      --notes) notes="$2"; shift 2 ;;
      --) shift; outputs+=("$@"); break ;;
      -*) ontarch_die "ontarch_emit_manifest: unknown flag $1" ;;
      *) outputs+=("$1"); shift ;;
    esac
  done
  [ -n "$id" ] && [ -n "$workflow" ] && [ -n "$tool" ] && [ -n "$retention" ] \
    || ontarch_die "ontarch_emit_manifest: --id --workflow --tool --retention required"
  [ "${#outputs[@]}" -gt 0 ] || ontarch_die "ontarch_emit_manifest: at least one output required"
  [ -n "$created_at" ] || created_at="$(date -u +%Y%m%d-%H%M%S)"

  local outs_json
  outs_json="$(printf '%s\n' "${outputs[@]}" | jq -R . | jq -sc .)"
  jq -n \
    --arg id "$id" \
    --arg workflow "$workflow" \
    --arg source "$source" \
    --arg created_at "$created_at" \
    --arg tool "$tool" \
    --arg retention "$retention" \
    --arg approved_to "$approved_to" \
    --arg tool_version "$tool_version" \
    --arg notes "$notes" \
    --argjson outputs "$outs_json" '{
      id: $id,
      workflow: $workflow,
      source: $source,
      created_at: $created_at,
      tool: $tool,
      outputs: $outputs,
      approved_to: (if $approved_to == "" then null else $approved_to end),
      retention: $retention
    }
    + (if $tool_version == "" then {} else {tool_version: $tool_version} end)
    + (if $notes == "" then {} else {notes: $notes} end)'
}

# ── Phase 1 machine-contract validators + generated-output install helpers ────
# Print a stable diagnostic code on stderr and return nonzero on failure.

_ontarch_diag() { printf '%s\n' "$*" >&2; }

# Guarded jq: always fail closed on jq process error (set -e is off).
# Args: <diag_prefix> <jq-args...> — last arg is the file/filter context via "$@".
# Usage: _ontarch_jq_err VARNAME diag_prefix -- jq args...
_ontarch_jq_r() {
  # Sets caller variable named $1 to jq -r output; returns jq status.
  local __outvar="$1"; shift
  local __out __st
  __out="$(jq -r "$@" 2>/dev/null)"
  __st=$?
  printf -v "$__outvar" '%s' "$__out"
  return "$__st"
}

# integer JSON Schema mirror: number with floor == .
_ontarch_jq_is_int='(type == "number" and . == floor)'

# Target-local temp under ONTARCH_REGISTRY (same filesystem as finals).
# Args: <name-hint>
ontarch_mktemp_registry() {
  local hint="${1:-tmp}"
  mkdir -p "$ONTARCH_REGISTRY" || return 1
  mktemp "$ONTARCH_REGISTRY/.${hint}.XXXXXX"
}

# Install two generated files with prior retention/rollback.
# Args: <src_a> <dest_a> <src_b> <dest_b>
ontarch_install_generated_pair() {
  local src_a="$1" dest_a="$2" src_b="$3" dest_b="$4"
  local bak_a="" bak_b=""
  [ -f "$src_a" ] && [ -f "$src_b" ] || return 1
  if [ -f "$dest_a" ]; then
    bak_a="$(ontarch_mktemp_registry bak-a)" || return 1
    cp -f "$dest_a" "$bak_a" || { rm -f "$bak_a"; return 1; }
  fi
  if [ -f "$dest_b" ]; then
    bak_b="$(ontarch_mktemp_registry bak-b)" || { rm -f "$bak_a"; return 1; }
    cp -f "$dest_b" "$bak_b" || { rm -f "$bak_a" "$bak_b"; return 1; }
  fi
  if ! mv -f "$src_a" "$dest_a"; then
    [ -n "$bak_a" ] && mv -f "$bak_a" "$dest_a"
    rm -f "$bak_a" "$bak_b"
    return 1
  fi
  if ! mv -f "$src_b" "$dest_b"; then
    [ -n "$bak_a" ] && mv -f "$bak_a" "$dest_a"
    [ -n "$bak_b" ] && mv -f "$bak_b" "$dest_b"
    rm -f "$bak_a" "$bak_b"
    return 1
  fi
  rm -f "$bak_a" "$bak_b"
  return 0
}

# Render BIN-INVENTORY.md from a validated inventory JSON path (stdout).
ontarch_render_bin_inventory_markdown() {
  local inv="$1"
  printf '# Bin inventory report\n\n'
  # shellcheck disable=SC2016
  printf 'GENERATED by `ontarch bin-report` (read-only). Regenerate: `moon run ontarch:bin-report`. Host-specific (gitignored).\n\n'
  printf 'generated_at: %s\n\n' "$(jq -r '.generated_at' "$inv")"
  printf 'summary: %s workflow dir(s) — %s with manifest\n\n' \
    "$(jq -r '.summary.total' "$inv")" \
    "$(jq -r '.summary.with_manifest' "$inv")"
  printf '| path | size | files | oldest_days | newest_days | manifest |\n'
  printf '|------|------|-------|-------------|-------------|----------|\n'
  jq -r '.workflows[] |
    (if .size_bytes >= 1073741824 then ((.size_bytes/1073741824)|floor|tostring) + "GiB"
     elif .size_bytes >= 1048576 then ((.size_bytes/1048576*10|floor)/10|tostring) + "MiB"
     elif .size_bytes >= 1024 then ((.size_bytes/1024*10|floor)/10|tostring) + "KiB"
     else (.size_bytes|tostring) + "B" end) as $sz |
    "| \(.path) | \($sz) | \(.file_count) | \(.oldest_file_age_days // "-") | \(.newest_file_age_days // "-") | \(if .manifest_present then "yes (\(.manifest_count))" else "no" end) |"' \
    "$inv"
}

# Args: <graph.json path> [schema]
ontarch_validate_graph_doc() {
  local graph="$1"
  local schema="${2:-$ONTARCH_GRAPHS/edges.schema.json}"
  [ -f "$graph" ] || { _ontarch_diag "graph:missing_file"; return 1; }
  [ -f "$schema" ] || { _ontarch_diag "graph:missing_schema"; return 1; }

  local err
  if ! err="$(jq -r '
    def utc: type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$");
    def no_ctrl: type == "string" and all(explode[]; . >= 32);
    if type != "object" then "graph:not_object"
    elif (keys | sort) != ["edges","generated_at","nodes","registry_generation"] then "graph:unknown_or_missing_root_field"
    elif (.generated_at | utc | not) then "graph:invalid_generated_at"
    elif (.registry_generation | type) != "object" then "graph:invalid_registry_generation"
    elif ((.registry_generation | keys | sort) != ["generated_at","source_fingerprints"]) then "graph:invalid_registry_generation"
    elif (.registry_generation.generated_at | utc | not) then "graph:invalid_generation_timestamp"
    else
      (.registry_generation.source_fingerprints // null) as $fps |
      if ($fps | type) != "array" then "graph:fingerprints_not_array"
      elif ($fps | length) != 4 then "graph:fingerprint_count"
      elif ($fps | map(.path) | unique | length) != 4 then "graph:duplicate_fingerprint_path"
      elif any($fps[]; (.path | type) != "string" or (.path | test("^/"))) then "graph:absolute_fingerprint_path"
      elif any($fps[]; .path | test("\\\\|\\.\\.")) then "graph:unsafe_fingerprint_path"
      elif any($fps[]; (.path | test("^registry/[a-z0-9][a-z0-9._-]*\\.json$") | not)) then "graph:bad_fingerprint_path"
      elif ($fps | map(.path) | sort) != ($fps | map(.path)) then "graph:fingerprint_paths_unsorted"
      elif any($fps[]; .algorithm != "sha256") then "graph:unsupported_fingerprint_algorithm"
      elif any($fps[]; (.digest | type) != "string" or (.digest | test("^[0-9a-f]{64}$") | not)) then "graph:malformed_fingerprint_digest"
      elif any($fps[]; ((keys | sort) != ["algorithm","digest","path"])) then "graph:unknown_fingerprint_field"
      elif ($fps | map(.path) | sort) != [
        "registry/policies.json","registry/profiles.json","registry/skills.json","registry/units.json"
      ] then "graph:fingerprint_path_set"
      elif (.nodes | type) != "array" or (.edges | type) != "array" then "graph:invalid_node_or_edge_shape"
      elif any(.nodes[]; ((keys | sort) != ["id","kind"]) or (.id | no_ctrl | not) or (.id | length) < 1) then "graph:invalid_node_or_edge_shape"
      elif any(.edges[]; ((keys | sort) != ["from","rel","to"])
        or (.from | no_ctrl | not) or (.to | no_ctrl | not) or (.rel | no_ctrl | not)) then "graph:invalid_node_or_edge_shape"
      elif any(.nodes[]; (.id | no_ctrl | not)) then "graph:control_character_id"
      elif any(.edges[]; (.from | no_ctrl | not) or (.to | no_ctrl | not)) then "graph:control_character_id"
      elif (.nodes | map(.id) | unique | length) != (.nodes | length) then "graph:duplicate_node_id"
      elif (.edges | map([.from,.rel,.to]) | unique | length) != (.edges | length) then "graph:duplicate_edge_tuple"
      elif (.nodes | sort_by(.kind, .id)) != .nodes then "graph:nodes_unsorted"
      elif (.edges | sort_by(.from, .rel, .to)) != .edges then "graph:edges_unsorted"
      else
        [.nodes[].id] as $ids |
        if any(.edges[]; (.from as $f | $ids | index($f) | not) or (.to as $t | $ids | index($t) | not))
        then "graph:dangling_edge_endpoint"
        else empty end
      end
    end
  ' "$graph")"; then
    _ontarch_diag "graph:jq_validation_error"
    return 1
  fi
  [ -z "$err" ] || { _ontarch_diag "$err"; return 1; }

  local name expected got reg_dir
  reg_dir="${ONTARCH_REGISTRY:-}"
  if [ -z "$reg_dir" ] || [ ! -d "$reg_dir" ]; then
    reg_dir="$(cd "$(dirname "$graph")" && pwd)"
  fi
  for name in policies.json profiles.json skills.json units.json; do
    [ -f "$reg_dir/$name" ] || { _ontarch_diag "graph:missing_upstream:$name"; return 1; }
    expected="$(_ontarch_sha256_file "$reg_dir/$name")" || return 1
    got="$(jq -r --arg p "registry/$name" \
      '.registry_generation.source_fingerprints[] | select(.path == $p) | .digest' "$graph")" || {
      _ontarch_diag "graph:jq_validation_error"; return 1
    }
    [ "$got" = "$expected" ] || { _ontarch_diag "graph:fingerprint_digest_mismatch:$name"; return 1; }
  done

  local node_kinds edge_rels k r
  node_kinds="$(jq -r '.properties.nodes.items.properties.kind.enum[]' "$schema")" || {
    _ontarch_diag "graph:jq_validation_error"; return 1
  }
  edge_rels="$(jq -r '.properties.edges.items.properties.rel.enum[]' "$schema")" || {
    _ontarch_diag "graph:jq_validation_error"; return 1
  }
  for k in $(jq -r '.nodes[].kind' "$graph" | sort -u); do
    echo "$node_kinds" | grep -qx "$k" || { _ontarch_diag "graph:invalid_node_kind:$k"; return 1; }
  done
  for r in $(jq -r '.edges[].rel' "$graph" | sort -u); do
    echo "$edge_rels" | grep -qx "$r" || { _ontarch_diag "graph:invalid_edge_rel:$r"; return 1; }
  done
  return 0
}

# Args: <inventory.json> [expected_root]
ontarch_validate_bin_inventory_doc() {
  local inv="$1"
  local expected_root="${2:-}"
  [ -f "$inv" ] || { _ontarch_diag "bin_inventory:missing_file"; return 1; }

  local err
  if ! err="$(jq -r '
    def isint: type == "number" and . == floor;
    def ageok:
      . == null or (isint and . >= 0 and . <= 365000);
    if type != "object" then "bin_inventory:schema"
    elif (keys | sort) != ["generated_at","root","summary","workflows"] then "bin_inventory:schema"
    elif (.generated_at | type != "string" or (test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$") | not))
      then "bin_inventory:schema"
    elif (.root | type != "string" or length < 1) then "bin_inventory:schema"
    elif ((.summary | keys | sort) != ["total","with_manifest"]) then "bin_inventory:schema"
    elif (.summary.total | isint | not) or .summary.total < 0 or .summary.total > 100000 then "bin_inventory:schema"
    elif (.summary.with_manifest | isint | not) or .summary.with_manifest < 0 or .summary.with_manifest > 100000
      then "bin_inventory:schema"
    elif .summary.with_manifest > .summary.total then "bin_inventory:schema"
    elif (.workflows | type) != "array" then "bin_inventory:schema"
    else
      (.workflows) as $w |
      if ($w | length) != .summary.total then "bin_inventory:summary_total_mismatch"
      elif ([ $w[] | select(.manifest_present == true) ] | length) != .summary.with_manifest
        then "bin_inventory:summary_manifest_mismatch"
      elif ($w | map(.path) | unique | length) != ($w | length) then "bin_inventory:duplicate_path"
      elif ($w | map(.path) | sort) != ($w | map(.path)) then "bin_inventory:unsorted_paths"
      elif any($w[]; ((keys | sort) != [
        "file_count","manifest_count","manifest_present","newest_file_age_days",
        "oldest_file_age_days","path","size_bytes"
      ])) then "bin_inventory:unknown_workflow_field"
      elif any($w[];
        (.path | type) != "string"
        or (.path | test("^/") )
        or (.path | test("\\\\|\\.\\.|\\n|\\r|\\t"))
        or (.path | test("^[^/]+/bin(/[^/]+)+$") | not)
        or (.path | test("(^|/)(lib|src)(/|$)"))
      ) then "bin_inventory:unsafe_path"
      elif any($w[];
        (.size_bytes | isint | not) or .size_bytes < 0 or .size_bytes > 1099511627776
        or (.file_count | isint | not) or .file_count < 0 or .file_count > 10000000
        or (.manifest_count | isint | not) or .manifest_count < 0 or .manifest_count > 100000
        or (.manifest_present | type) != "boolean"
        or (.manifest_present != (.manifest_count > 0))
        or (.oldest_file_age_days | ageok | not)
        or (.newest_file_age_days | ageok | not)
      ) then "bin_inventory:count_inconsistency"
      elif any($w[];
        .oldest_file_age_days != null and .newest_file_age_days != null
        and .newest_file_age_days > .oldest_file_age_days
      ) then "bin_inventory:age_inconsistency"
      else empty end
    end
  ' "$inv")"; then
    _ontarch_diag "bin_inventory:jq_validation_error"
    return 1
  fi
  [ -z "$err" ] || { _ontarch_diag "$err"; return 1; }

  if [ -n "$expected_root" ]; then
    local root
    root="$(jq -r '.root' "$inv")" || { _ontarch_diag "bin_inventory:jq_validation_error"; return 1; }
    [ "$root" = "$expected_root" ] || { _ontarch_diag "bin_inventory:root_mismatch"; return 1; }
  fi
  return 0
}

# Closed cleanup reason vocabulary (Phase 1).
_ONTARCH_CLEANUP_REASONS='["approved","approved-to-mismatch","approved-to-null","current","invalid-manifest","lib-or-src","multiple-manifests","no-manifest","outside-scope","retention-permanent","retention-review-required","scope-required","stale"]'

# Args: <cleanup-plan.json>
ontarch_validate_bin_cleanup_plan_doc() {
  local plan="$1"
  [ -f "$plan" ] || { _ontarch_diag "bin_cleanup:missing_file"; return 1; }

  local err
  if ! err="$(jq -r --argjson reasons "$_ONTARCH_CLEANUP_REASONS" '
    def isint: type == "number" and . == floor;
    def pathok:
      type == "string" and length > 0
      and (test("^/") | not)
      and (test("\\\\|\\.\\.|\\n|\\r|\\t") | not)
      and test("^[^/]+/bin(/[^/]+)+$");
    def retentionok:
      . == null
      or . == "review-before-delete"
      or . == "permanent"
      or . == "session-exports"
      or (type == "string" and test("^auto-archive-after:[0-9]+d$"));
    if type != "object" then "bin_cleanup:schema"
    elif (keys | sort) != [
      "entries","generated_at","inventory_generated_at","inventory_refreshed",
      "mode","mutation_executed","scope","summary"
    ] then "bin_cleanup:schema"
    elif .mutation_executed != false then "bin_cleanup:mutation_executed"
    elif (.mode | IN("report-only","dry-run","archive","delete-approved") | not) then "bin_cleanup:schema"
    elif (.inventory_refreshed | type) != "boolean" then "bin_cleanup:schema"
    elif (.generated_at | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$") | not)
      then "bin_cleanup:schema"
    elif (.inventory_generated_at | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$") | not)
      then "bin_cleanup:schema"
    elif .scope != null and (.scope | pathok | not) then "bin_cleanup:unsafe_scope"
    elif ((.summary | keys | sort) != ["advisory","blocked","total","would_archive","would_delete"])
      then "bin_cleanup:schema"
    elif any(.summary[]?; (isint | not) or . < 0 or . > 100000) then "bin_cleanup:schema"
    elif (.entries | type) != "array" then "bin_cleanup:schema"
    else
      (.entries) as $e |
      if ($e | length) != .summary.total then "bin_cleanup:summary_total_mismatch"
      elif ([ $e[] | select(.disposition == "advisory") ] | length) != .summary.advisory
        then "bin_cleanup:summary_advisory_mismatch"
      elif ([ $e[] | select(.disposition == "would_archive") ] | length) != .summary.would_archive
        then "bin_cleanup:summary_would_archive_mismatch"
      elif ([ $e[] | select(.disposition == "would_delete") ] | length) != .summary.would_delete
        then "bin_cleanup:summary_would_delete_mismatch"
      elif ([ $e[] | select(.disposition == "blocked") ] | length) != .summary.blocked
        then "bin_cleanup:summary_blocked_mismatch"
      elif ($e | map(.path) | unique | length) != ($e | length) then "bin_cleanup:duplicate_path"
      elif ($e | map(.path) | sort) != ($e | map(.path)) then "bin_cleanup:unsorted_paths"
      elif any($e[]; ((keys | sort) != [
        "approved_to_matches","disposition","path","reason","retention"
      ])) then "bin_cleanup:unknown_entry_field"
      elif any($e[];
        (.disposition | IN("advisory","would_archive","would_delete","blocked") | not)
        or (.reason | IN($reasons[]) | not)
        or (.path | pathok | not)
        or (.retention | retentionok | not)
        or ((.approved_to_matches != null) and (.approved_to_matches | type) != "boolean")
      ) then "bin_cleanup:invalid_entry"
      elif .scope != null and (
        (.scope) as $scope | any($e[]; (.path != $scope) and (.path | startswith($scope + "/") | not))
      ) then "bin_cleanup:scope_violation"
      else empty end
    end
  ' "$plan")"; then
    _ontarch_diag "bin_cleanup:jq_validation_error"
    return 1
  fi
  [ -z "$err" ] || { _ontarch_diag "$err"; return 1; }
  return 0
}

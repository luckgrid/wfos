#!/usr/bin/env bash
# Ontarch registry emitters: generate the registry JSON from descriptors, policies, and the
# .agents/ navigation layer. Sourced after common.sh + descriptor.sh (uses jq).
# Output is compact (one record per line, like tools.json) so RTK + jq stay cheap.

_ontarch_now() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Join a compact JSON array into a single inline `a,b,c` string (empty if no elements).
_ontarch_inline() { jq -c '.[]' <<<"$1" | paste -sd, -; }

# units.json — colocated-first discovery, central overrides colocated for a shared id.
ontarch_emit_units() {
  local units='[]' f src full id rec
  local -A from_central=()
  while IFS=$'\037' read -r f src; do
    [ -n "$f" ] || continue
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
  local summary inline
  summary="$(jq -c '{
    total: length,
    by_kind: (group_by(.kind) | map({key: .[0].kind, value: length}) | from_entries)
  }' <<<"$units")"
  inline="$(_ontarch_inline "$units")"
  printf '{\n  "generated_at": "%s",\n  "summary": %s,\n  "units": [%s]\n}\n' \
    "$(_ontarch_now)" "$summary" "$inline"
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
  local total clean dirty
  total="$(jq 'length' <<<"$arr")"
  clean="$(jq '[.[] | select(.worktree_status.state == "clean")] | length' <<<"$arr")"
  dirty="$(jq '[.[] | select(.worktree_status.state == "dirty")] | length' <<<"$arr")"
  jq -n --arg ts "$(_ontarch_now)" --arg root "$WORKSPACES_DIR" \
    --argjson total "$total" --argjson clean "$clean" --argjson dirty "$dirty" \
    --argjson ws "$arr" '{
      generated_at: $ts, root: $root,
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
  [ -f "$profiles" ] || printf '{"profiles":[]}' > "$profiles"
  [ -f "$skills" ]   || printf '{"skills":[]}' > "$skills"

  jq -n --arg ts "$(_ontarch_now)" \
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
      generated_at: $ts,
      nodes: ($unit_nodes + $cap_nodes + $policy_nodes + $actor_nodes + $profile_nodes + $skill_nodes),
      edges: ($provides_edges + $requires_edges + $uses_edges + $governed_edges + $blocked_edges + $selects_edges + $can_invoke_edges)
    }
  '
}

# graph.dot — Graphviz DOT rendering, derived from graph.json (read from stdin).
ontarch_emit_graph_dot() {
  jq -r '"digraph ontarch {\n  rankdir=LR;\n  node [shape=box];\n",
         (.edges[] | "  \"\(.from)\" -> \"\(.to)\" [label=\"\(.rel)\"];\n"),
         "}\n"'
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
    session_log_target: (.logs.session_log_target // null)
  }' <<<"$1"
}

# profiles.json — populated by E05 from .agents/profiles/*.toml. Each profile is read by the
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
    # BSD/macOS: -f %m; GNU: -c %Y
    mtime="$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || echo "")"
    [ -n "$mtime" ] || continue
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

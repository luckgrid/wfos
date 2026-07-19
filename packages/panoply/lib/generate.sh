#!/usr/bin/env bash
# Panoply install-artifact generators: derive a Brewfile and a mise [tools] block from the
# manifest (the single source of truth). Pure stdout, dry-run, read-only — agent-safe.
# Sourced after common.sh + manifest.sh.

# Emit a Brewfile from the manifest. Every tool with a non-empty `brew` field, grouped by
# module in manifest order. Optional filters:
#   $1 == "--missing"  -> only tools not currently detected on this host
#   $2 == "--defaults" -> only tools with default = true
# (filters compose; order is fixed: [--missing] [--defaults])
# shellcheck disable=SC2120  # callable with or without filter args
panoply_gen_brewfile() {
  local only_missing=0 only_defaults=0 a
  for a in "$@"; do
    case "$a" in
      --missing)  only_missing=1 ;;
      --defaults) only_defaults=1 ;;
    esac
  done

  printf '%s\n' "# Panoply tool library — Homebrew bundle (GENERATED from manifest/panoply.tools.toml)."
  printf '%s\n' "# Regenerate with:  panoply gen brewfile   (do not hand-edit — the manifest is the source of truth)."
  printf '%s\n' "# Install all with:  brew bundle --file=Brewfile"
  printf '%s\n' "# \`panoply bootstrap\` installs only the missing subset."

  local current=""
  # shellcheck disable=SC2034  # fields bound positionally; not all used here
  while IFS=$'\037' read -r module id def brew detect agent_safe alts purpose; do
    [ -n "$id" ] || continue
    [ -n "$brew" ] || continue
    [ "$only_defaults" = "1" ] && [ "$def" != "true" ] && continue
    if [ "$only_missing" = "1" ] && panoply_detect "$detect"; then continue; fi
    if [ "$module" != "$current" ]; then
      printf '\n# %s\n' "$module"
      current="$module"
    fi
    printf 'brew "%s"\n' "$brew"
  done < <(panoply_manifest_tsv)
}

# Emit a mise [tools] block for mise-eligible Panoply runtimes (detect-only js/rust tools).
# Commented by default to honor the canon rule: the manifest describes Panoply's desired tool
# SET, not language-runtime versions (those are pinned by the operator via mise/proto).
panoply_gen_mise() {
  printf '%s\n' "# Panoply mise [tools] block (GENERATED from manifest/panoply.tools.toml)."
  printf '%s\n' "# Runtimes are listed commented: the manifest names the desired tool set, not versions."
  printf '%s\n' "# Uncomment and pin a version, then \`mise install\`. Native version files stay authoritative."
  printf '%s\n' "[tools]"
  # shellcheck disable=SC2034  # fields bound positionally; not all used here
  while IFS=$'\037' read -r module id def brew detect agent_safe alts purpose; do
    [ -n "$id" ] || continue
    [ -n "$brew" ] && continue          # brew-installed tools are not mise-managed here
    case "$module" in js|rust) ;; *) continue ;; esac
    case "$detect" in */*) continue ;; esac   # skip path-detected entries
    printf '# %s = "latest"\n' "$detect"
  done < <(panoply_manifest_tsv)
}

# Compare the generated Brewfile against the committed config/Brewfile. Exit 0 if identical,
# 1 on drift (prints a unified diff). This is what makes "manifest is the source of truth"
# enforceable as a gate.
panoply_gen_brewfile_check() {
  local committed="$PANOPLY_CONFIG/Brewfile"
  [ -f "$committed" ] || { panoply_err "missing committed Brewfile: $committed"; return 1; }
  # shellcheck disable=SC2119  # no filter: --check compares the full derived Brewfile
  if diff -u "$committed" <(panoply_gen_brewfile) >/dev/null 2>&1; then
    panoply_ok "Brewfile matches manifest"
    return 0
  fi
  panoply_err "Brewfile drift — regenerate with: panoply gen brewfile > config/Brewfile"
  # shellcheck disable=SC2119
  diff -u "$committed" <(panoply_gen_brewfile) || true
  return 1
}

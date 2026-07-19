#!/usr/bin/env bash
# Panoply per-module configuration: symlink config templates into the home tree and
# wire the shell. Config-only — no tool installation. Idempotent. Sourced after common.sh.

# Symlink repo config -> home target, backing up any existing real file once.
panoply_link_config() {
  local src="$1" dest="$2"
  [ -f "$src" ] || { panoply_warn "missing config source: $src"; return 1; }
  mkdir -p "$(dirname "$dest")"
  if [ -L "$dest" ]; then
    local cur; cur="$(_panoply_realpath "$dest")"
    [ "$cur" = "$(_panoply_realpath "$src")" ] && { panoply_ok "linked $dest"; return 0; }
    rm -f "$dest"
  elif [ -e "$dest" ]; then
    local bak
    bak="$dest.pre-panoply.$(date +%Y%m%d%H%M%S)"
    mv "$dest" "$bak"
    panoply_warn "backed up existing $dest -> $bak"
  fi
  ln -s "$src" "$dest"
  panoply_ok "linked $dest -> $src"
}

panoply_configure_shell()   { panoply_link_config "$PANOPLY_CONFIG/starship.toml" "$HOME/.config/starship.toml"; }
panoply_configure_session() {
  panoply_link_config "$PANOPLY_CONFIG/tmux.conf" "$HOME/.config/tmux/tmux.conf"
}
panoply_configure_tools()   { panoply_link_config "$PANOPLY_CONFIG/mise/config.toml" "$HOME/.config/mise/config.toml"; }

# Put `panoply` (and friends) on PATH via ~/.local/bin (already on this machine's PATH).
panoply_link_cli() {
  local target_dir="$HOME/.local/bin"
  mkdir -p "$target_dir"
  local c
  for c in panoply panoply-doctor panoply-bootstrap panoply-env; do
    ln -sf "$PANOPLY_BIN/$c" "$target_dir/$c"
  done
  panoply_ok "linked panoply CLI into $target_dir"
}

# Append a single guarded source line to ~/.zshrc (backed up once).
panoply_wire_shell() {
  local rc="$HOME/.zshrc"
  local fragment="$PANOPLY_CONFIG/shell/panoply.zsh"
  local marker="# WfOS Panoply"
  [ -f "$fragment" ] || { panoply_warn "missing shell fragment: $fragment"; return 1; }
  if [ -f "$rc" ] && grep -qF "$marker" "$rc"; then
    panoply_ok "shell already wired in $rc"
    return 0
  fi
  if [ -f "$rc" ]; then
    cp "$rc" "$rc.pre-panoply.$(date +%Y%m%d%H%M%S)"
  fi
  {
    printf '\n%s\n' "$marker"
    printf '%s\n' "export PANOPLY_HOME=\"$PANOPLY_PKG\""
    printf '%s\n' "[ -f \"$fragment\" ] && source \"$fragment\""
  } >> "$rc"
  panoply_ok "wired Panoply shell fragment into $rc"
}

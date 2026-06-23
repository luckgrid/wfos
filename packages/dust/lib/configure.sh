#!/usr/bin/env bash
# Dust per-module configuration: symlink config templates into the home tree and
# wire the shell. Config-only — no tool installation. Idempotent. Sourced after common.sh.

# Symlink repo config -> home target, backing up any existing real file once.
dust_link_config() {
  local src="$1" dest="$2"
  [ -f "$src" ] || { dust_warn "missing config source: $src"; return 1; }
  mkdir -p "$(dirname "$dest")"
  if [ -L "$dest" ]; then
    local cur; cur="$(_dust_realpath "$dest")"
    [ "$cur" = "$(_dust_realpath "$src")" ] && { dust_ok "linked $dest"; return 0; }
    rm -f "$dest"
  elif [ -e "$dest" ]; then
    local bak
    bak="$dest.pre-dust.$(date +%Y%m%d%H%M%S)"
    mv "$dest" "$bak"
    dust_warn "backed up existing $dest -> $bak"
  fi
  ln -s "$src" "$dest"
  dust_ok "linked $dest -> $src"
}

dust_configure_shell()   { dust_link_config "$DUST_CONFIG/starship.toml" "$HOME/.config/starship.toml"; }
dust_configure_session() {
  dust_link_config "$DUST_CONFIG/tmux.conf" "$HOME/.config/tmux/tmux.conf"
}
dust_configure_tools()   { dust_link_config "$DUST_CONFIG/mise/config.toml" "$HOME/.config/mise/config.toml"; }

# Put `dust` (and friends) on PATH via ~/.local/bin (already on this machine's PATH).
dust_link_cli() {
  local target_dir="$HOME/.local/bin"
  mkdir -p "$target_dir"
  local c
  for c in dust dust-doctor dust-bootstrap dust-env; do
    ln -sf "$DUST_BIN/$c" "$target_dir/$c"
  done
  dust_ok "linked dust CLI into $target_dir"
}

# Append a single guarded source line to ~/.zshrc (backed up once).
dust_wire_shell() {
  local rc="$HOME/.zshrc"
  local fragment="$DUST_CONFIG/shell/dust.zsh"
  local marker="# WfOS Dust"
  [ -f "$fragment" ] || { dust_warn "missing shell fragment: $fragment"; return 1; }
  if [ -f "$rc" ] && grep -qF "$marker" "$rc"; then
    dust_ok "shell already wired in $rc"
    return 0
  fi
  if [ -f "$rc" ]; then
    cp "$rc" "$rc.pre-dust.$(date +%Y%m%d%H%M%S)"
  fi
  {
    printf '\n%s\n' "$marker"
    printf '%s\n' "export DUST_HOME=\"$DUST_PKG\""
    printf '%s\n' "[ -f \"$fragment\" ] && source \"$fragment\""
  } >> "$rc"
  dust_ok "wired Dust shell fragment into $rc"
}

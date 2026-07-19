# WfOS Panoply — shell activation fragment (zsh).
# Sourced from ~/.zshrc by `panoply bootstrap`. Every activation is guarded so this
# file is safe to source even when a tool is not installed.

# Panoply home + CLI on PATH (also symlinked into ~/.local/bin during bootstrap).
# Default layout is a suggestion (Workstreams/Build/…); override by exporting
# PANOPLY_HOME in ~/.zshenv or before sourcing — e.g. when wfos lives elsewhere.
export PANOPLY_HOME="${PANOPLY_HOME:-$HOME/Workstreams/Build/src/workspaces/wfos/packages/panoply}"
case ":$PATH:" in
  *":$PANOPLY_HOME/bin:"*) ;;
  *) export PATH="$PANOPLY_HOME/bin:$PATH" ;;
esac

# Tool version manager (Panoply default). Activated after proto so mise manages
# Panoply-scoped runtimes; proto remains available for existing workflows.
command -v mise >/dev/null 2>&1 && eval "$(mise activate zsh)"

# Per-directory environments.
command -v direnv >/dev/null 2>&1 && eval "$(direnv hook zsh)"

# Prompt.
command -v starship >/dev/null 2>&1 && eval "$(starship init zsh)"

# Smarter cd.
command -v zoxide >/dev/null 2>&1 && eval "$(zoxide init zsh)"

# Fuzzy finder key bindings + completion (fzf >= 0.48).
if command -v fzf >/dev/null 2>&1; then
  source <(fzf --zsh) 2>/dev/null || true
fi

# Modern coreutils-style aliases when available.
command -v eza >/dev/null 2>&1 && alias ls='eza' && alias ll='eza -l --git' && alias la='eza -la --git'
command -v bat >/dev/null 2>&1 && alias cat='bat --paging=never'

# RTK output-compression layer (recommended-default; swappable via PANOPLY_RTK).
# Skipped when the chezmoi layer manages it (PANOPLY_RTK_MANAGED=1) to avoid double-sourcing;
# the chezmoi fragment sets PANOPLY_RTK from profile data, then sources this same file.
if [ -z "${PANOPLY_RTK_MANAGED:-}" ]; then
  _panoply_rtk_frag="${PANOPLY_HOME:-$HOME/Workstreams/Build/src/workspaces/wfos/packages/panoply}/config/shell/rtk.zsh"
  [ -f "$_panoply_rtk_frag" ] && source "$_panoply_rtk_frag"
  unset _panoply_rtk_frag
fi

# Zsh plugins (Homebrew, sourced files — guarded so missing plugins are harmless).
# Order matters: autosuggestions/autocomplete first, syntax-highlighting LAST.
# Skipped when the chezmoi plugin layer manages plugins — it sets PANOPLY_PLUGINS_MANAGED=1
# and sources its own profile-aware fragment, so this block stands down to avoid double-sourcing.
if [ -z "${PANOPLY_PLUGINS_MANAGED:-}" ]; then
  _panoply_brew_prefix="${HOMEBREW_PREFIX:-$(brew --prefix 2>/dev/null)}"
  if [ -n "$_panoply_brew_prefix" ]; then
    [ -f "$_panoply_brew_prefix/share/zsh-autosuggestions/zsh-autosuggestions.zsh" ] && \
      source "$_panoply_brew_prefix/share/zsh-autosuggestions/zsh-autosuggestions.zsh"
    # zsh-autocomplete is optional and can conflict with other completion setups.
    [ -f "$_panoply_brew_prefix/share/zsh-autocomplete/zsh-autocomplete.plugin.zsh" ] && \
      source "$_panoply_brew_prefix/share/zsh-autocomplete/zsh-autocomplete.plugin.zsh"
    # Must be sourced last to wrap the final widget set.
    [ -f "$_panoply_brew_prefix/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh" ] && \
      source "$_panoply_brew_prefix/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"
  fi
  unset _panoply_brew_prefix
fi

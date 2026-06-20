# WfOS Dust — shell activation fragment (zsh).
# Sourced from ~/.zshrc by `dust bootstrap`. Every activation is guarded so this
# file is safe to source even when a tool is not installed.

# Dust home + CLI on PATH (also symlinked into ~/.local/bin during bootstrap).
export DUST_HOME="${DUST_HOME:-$HOME/Workstreams/Tech/Dev/src/workspaces/wfos/packages/dust}"
case ":$PATH:" in
  *":$DUST_HOME/bin:"*) ;;
  *) export PATH="$DUST_HOME/bin:$PATH" ;;
esac

# Tool version manager (Dust default). Activated after proto so mise manages
# Dust-scoped runtimes; proto remains available for existing workflows.
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

# Zsh plugins (Homebrew, sourced files — guarded so missing plugins are harmless).
# Order matters: autosuggestions/autocomplete first, syntax-highlighting LAST.
_dust_brew_prefix="${HOMEBREW_PREFIX:-$(brew --prefix 2>/dev/null)}"
if [ -n "$_dust_brew_prefix" ]; then
  [ -f "$_dust_brew_prefix/share/zsh-autosuggestions/zsh-autosuggestions.zsh" ] && \
    source "$_dust_brew_prefix/share/zsh-autosuggestions/zsh-autosuggestions.zsh"
  # zsh-autocomplete is optional and can conflict with other completion setups.
  [ -f "$_dust_brew_prefix/share/zsh-autocomplete/zsh-autocomplete.plugin.zsh" ] && \
    source "$_dust_brew_prefix/share/zsh-autocomplete/zsh-autocomplete.plugin.zsh"
  # Must be sourced last to wrap the final widget set.
  [ -f "$_dust_brew_prefix/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh" ] && \
    source "$_dust_brew_prefix/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"
fi
unset _dust_brew_prefix

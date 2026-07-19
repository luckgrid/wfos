# WfOS Panoply — RTK output-compression layer (zsh).
# RTK (https://github.com/rtk-ai/rtk) is the recommended-default LLM output compressor: it
# proxies dev commands and rewrites their output to cut tokens 60-90%. It is SWAPPABLE — this
# whole layer is a no-op unless RTK is installed AND PANOPLY_RTK=1 (default). Set PANOPLY_RTK=0 to
# disable, or replace it in profile data (see dotfiles/.chezmoidata/profiles.toml `rtk`).
#
# Safety: we route ONLY read-only / high-output subcommands through rtk and never shadow
# interactive or mutating commands. `command <tool>` always remains the raw escape hatch.

# Default on; the chezmoi profile layer sets this explicitly per profile.
: "${PANOPLY_RTK:=1}"

# Enable only when: routing requested, rtk on PATH, and it is the *token-killer* rtk (which
# exposes a `gain` subcommand) — not reachingforthejack/rtk (Rust Type Kit), which does not.
if [ "${PANOPLY_RTK}" = "1" ] && command -v rtk >/dev/null 2>&1 && rtk gain --help >/dev/null 2>&1; then

  # git: compress only read-only subcommands; everything else (commit, push, rebase, …) runs raw.
  git() {
    case "${1:-}" in
      status|diff|log|show|branch|blame) rtk git "$@" ;;
      *) command git "$@" ;;
    esac
  }

  # High-output read tools: route through rtk, which falls back to the native tool.
  grep() { rtk grep "$@"; }
  rg()   { rtk grep "$@"; }

  # Convenience: explicit, non-shadowing helpers.
  alias rtk-gain='rtk gain'
  panoply-rtk() { rtk "$@"; }    # raw passthrough to the proxy

  export PANOPLY_RTK_ACTIVE=1
fi

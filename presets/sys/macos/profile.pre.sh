# Managed by `shine sys bootstrap` for macOS. Existing user config is left untouched.

# Keep terminal-aware tools aligned with the terminal used for this session.
# OSC/PTY/timeout/RGB parsing lives entirely in the shine binary — see
# `shine theme sync` and docs/terminal-theme-sync-prd.md. This block must
# stay thin: it only decides whether to call the binary at all, never how to
# talk to the terminal.
if [[ "${SHINE_SYNC_TERMINAL_THEME:-1}" != "0" ]] &&
   command -v shine >/dev/null 2>&1; then
  eval "$(shine theme sync --auto --quiet 2>/dev/null)"
fi

# Homebrew prefix cache
if [[ -d "/opt/homebrew" ]]; then
  export HOMEBREW_PREFIX="/opt/homebrew"
elif [[ -x "/usr/local/bin/brew" ]]; then
  export HOMEBREW_PREFIX="/usr/local"
else
  export HOMEBREW_PREFIX=""
fi

# Homebrew zsh completions
if [[ -n "${ZSH_VERSION:-}" && -n "$HOMEBREW_PREFIX" && -d "$HOMEBREW_PREFIX/share/zsh/site-functions" ]]; then
  typeset -U fpath
  fpath=("$HOMEBREW_PREFIX/share/zsh/site-functions" $fpath)
fi

# Basic PATH
typeset -U path PATH
path=(
  "$HOME/bin"
  "$HOME/.local/bin"
  "$HOME/.cargo/bin"
  "/usr/local/bin"
  "/usr/local/sbin"
  "/opt/homebrew/bin"
  "/opt/homebrew/sbin"
  $path
)
export PATH

# nvm
export NVM_DIR="$HOME/.nvm"

# Bun
export BUN_INSTALL="$HOME/.bun"
if [[ -d "$BUN_INSTALL/bin" ]]; then
  path=("$BUN_INSTALL/bin" $path)
fi

# pnpm
export PNPM_HOME="$HOME/Library/pnpm"
case ":$PATH:" in
  *":$PNPM_HOME/bin:"*) ;;
  *) path=("$PNPM_HOME/bin" $path) ;;
esac

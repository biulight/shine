# Managed by `shine sys init` for macOS. Existing user config is left untouched.

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
if [[ -d "$PNPM_HOME" ]]; then
  case ":$PATH:" in
    *":$PNPM_HOME:"*) ;;
    *) path=("$PNPM_HOME" $path) ;;
  esac
fi
if [[ -d "$PNPM_HOME/bin" ]]; then
  case ":$PATH:" in
    *":$PNPM_HOME/bin:"*) ;;
    *) path=("$PNPM_HOME/bin" $path) ;;
  esac
fi

# Managed by `shine sys init` for Ubuntu. Existing user config is left untouched.

shine_ubuntu_sys_shell="${SHINE_UBUNTU_SYS_SHELL:-bash}"

# User-local binaries
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) export PATH="$HOME/.local/bin:$PATH" ;;
esac

# Homebrew
if [[ -x "/home/linuxbrew/.linuxbrew/bin/brew" ]]; then
  eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"
elif [[ -x "$HOME/.linuxbrew/bin/brew" ]]; then
  eval "$("$HOME/.linuxbrew/bin/brew" shellenv)"
fi

# Homebrew zsh completions
if [[ "${shine_ubuntu_sys_shell}" == "zsh" && -n "${ZSH_VERSION:-}" && -n "${HOMEBREW_PREFIX:-}" && -d "$HOMEBREW_PREFIX/share/zsh/site-functions" ]]; then
  typeset -U fpath
  fpath=("$HOMEBREW_PREFIX/share/zsh/site-functions" $fpath)
fi

# pnpm
export PNPM_HOME="$HOME/.local/share/pnpm"
case ":$PATH:" in
  *":$PNPM_HOME:"*) ;;
  *) export PATH="$PNPM_HOME:$PATH" ;;
esac
if [[ -d "$PNPM_HOME/bin" ]]; then
  case ":$PATH:" in
    *":$PNPM_HOME/bin:"*) ;;
    *) export PATH="$PNPM_HOME/bin:$PATH" ;;
  esac
fi

# Atuin environment
if [[ -f "$HOME/.atuin/bin/env" ]]; then
  . "$HOME/.atuin/bin/env"
fi

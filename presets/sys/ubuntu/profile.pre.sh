# Managed by `shine sys bootstrap` for Ubuntu. Existing user config is left untouched.

shine_ubuntu_sys_shell="${SHINE_UBUNTU_SYS_SHELL:-bash}"

# Keep terminal-aware tools aligned with the local terminal used for this
# session. OSC/PTY/timeout/RGB parsing lives entirely in the shine binary —
# see `shine theme sync` and docs/terminal-theme-sync-prd.md. This block must
# stay thin: it only decides whether to call the binary at all, never how to
# talk to the terminal.
if [[ "${SHINE_SYNC_TERMINAL_THEME:-1}" != "0" ]] &&
   command -v shine >/dev/null 2>&1; then
  eval "$(shine theme sync --auto --quiet 2>/dev/null)"
fi

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

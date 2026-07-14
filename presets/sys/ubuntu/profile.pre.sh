# Managed by `shine sys init` for Ubuntu. Existing user config is left untouched.

shine_ubuntu_sys_shell="${SHINE_UBUNTU_SYS_SHELL:-bash}"

# Keep terminal-aware tools aligned with the local terminal used for this session.
shine_apply_terminal_theme() {
  local response="$1" char rgb red green blue
  local red_value green_value blue_value

  case "$response" in
    *$'\033]11;rgb:'*$'\033\\') ;;
    *) return ;;
  esac
  rgb="${response#*$'\033]11;rgb:'}"
  rgb="${rgb%%$'\033\\'*}"
  red="${rgb%%/*}"
  rgb="${rgb#*/}"
  green="${rgb%%/*}"
  blue="${rgb#*/}"

  for char in "$red" "$green" "$blue"; do
    [[ -n "$char" && ${#char} -le 4 && "$char" != *[!0-9a-fA-F]* ]] || return
  done

  red_value=$((16#$red * 255 / (16 ** ${#red} - 1)))
  green_value=$((16#$green * 255 / (16 ** ${#green} - 1)))
  blue_value=$((16#$blue * 255 / (16 ** ${#blue} - 1)))
  if (( 299 * red_value + 587 * green_value + 114 * blue_value >= 128000 )); then
    export SHINE_TERMINAL_THEME="light"
    export BAT_THEME="${SHINE_BAT_LIGHT_THEME:-GitHub}"
  else
    export SHINE_TERMINAL_THEME="dark"
    export BAT_THEME="${SHINE_BAT_DARK_THEME:-OneHalfDark}"
  fi
}

shine_sync_terminal_theme() {
  case "$-" in
    *i*) ;;
    *) return ;;
  esac
  [[ "${SHINE_SYNC_TERMINAL_THEME:-1}" == "0" ]] && return
  [[ -r /dev/tty && -w /dev/tty ]] || return
  [[ -n "${BASH_VERSION:-}" || -n "${ZSH_VERSION:-}" ]] || return

  local response="" char="" read_timeout="0.15" count=0 tty_state=""

  # Terminal responses to OSC queries arrive as input bytes.  Disable tty echo
  # while reading them, otherwise SSH echoes the response payload as if the
  # user had typed it (for example: `11;rgb:0f0f/1616/1010`).
  tty_state=$(stty -g < /dev/tty 2>/dev/null) || return
  [[ -n "$tty_state" ]] || return
  stty -echo < /dev/tty 2>/dev/null || return

  if ! printf '\033]11;?\033\\' > /dev/tty 2>/dev/null; then
    stty "$tty_state" < /dev/tty 2>/dev/null || true
    return
  fi
  while (( count < 64 )); do
    char=""
    if [[ -n "${BASH_VERSION:-}" ]]; then
      IFS= read -r -s -n 1 -t "$read_timeout" char < /dev/tty 2>/dev/null || break
    else
      IFS= read -r -s -k 1 -t "$read_timeout" char < /dev/tty 2>/dev/null || break
    fi
    response+="$char"
    (( count += 1 ))
    [[ "$response" == *$'\033\\' ]] && break
    read_timeout="0.01"
  done
  stty "$tty_state" < /dev/tty 2>/dev/null || true
  shine_apply_terminal_theme "$response"
}

shine_sync_terminal_theme
unset -f shine_apply_terminal_theme shine_sync_terminal_theme 2>/dev/null

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

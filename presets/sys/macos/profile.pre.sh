# Managed by `shine sys init` for macOS. Existing user config is left untouched.

# Keep terminal-aware tools aligned with the terminal used for this session.
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

  local response="" char="" read_timeout="0.15" count=0

  printf '\033]11;?\033\\' > /dev/tty 2>/dev/null || return
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
  shine_apply_terminal_theme "$response"
}

shine_sync_terminal_theme
unset -f shine_apply_terminal_theme shine_sync_terminal_theme 2>/dev/null

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

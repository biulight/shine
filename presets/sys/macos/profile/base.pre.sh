# Managed by `shine sys bootstrap` for macOS. Existing user config is left untouched.

if [[ "${SHINE_SYNC_TERMINAL_THEME:-1}" != "0" ]] &&
   command -v shine >/dev/null 2>&1; then
  eval "$(shine theme sync --auto --quiet 2>/dev/null)"
fi

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

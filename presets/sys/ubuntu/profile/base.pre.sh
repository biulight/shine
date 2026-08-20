# Managed by `shine sys bootstrap` for Ubuntu. Existing user config is left untouched.

shine_ubuntu_sys_shell="${SHINE_UBUNTU_SYS_SHELL:-bash}"

if [[ "${SHINE_SYNC_TERMINAL_THEME:-1}" != "0" ]] &&
   command -v shine >/dev/null 2>&1; then
  eval "$(shine theme sync --auto --quiet 2>/dev/null)"
fi

case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) export PATH="$HOME/.local/bin:$PATH" ;;
esac

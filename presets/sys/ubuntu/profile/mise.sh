if command -v mise >/dev/null 2>&1; then
  eval "$(mise activate "${shine_ubuntu_sys_shell}")"
elif [[ -x "$HOME/.local/bin/mise" ]]; then
  eval "$("$HOME/.local/bin/mise" activate "${shine_ubuntu_sys_shell}")"
fi

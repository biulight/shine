# Managed by `shine sys init` for Ubuntu. Existing user config is left untouched.

shine_ubuntu_sys_shell="${SHINE_UBUNTU_SYS_SHELL:-bash}"

# eza
if command -v eza >/dev/null 2>&1; then
  alias ls='eza --icons'
  alias ll='eza -la --icons'
  alias lt='eza --tree'
fi

# bat
if command -v bat >/dev/null 2>&1; then
  alias cat='bat'
fi

# Yazi
if command -v yazi >/dev/null 2>&1; then
  y() {
    local tmp cwd
    tmp="$(mktemp -t "yazi-cwd.XXXXXX")"
    command yazi "$@" --cwd-file="$tmp"
    cwd="$(cat "$tmp")"
    [[ "$cwd" != "$PWD" && -d "$cwd" ]] && builtin cd -- "$cwd"
    command rm -f -- "$tmp"
  }
fi

# fzf
if command -v fzf >/dev/null 2>&1; then
  if fzf --"${shine_ubuntu_sys_shell}" >/dev/null 2>&1; then
    eval "$(fzf --"${shine_ubuntu_sys_shell}")"
  elif [[ "${shine_ubuntu_sys_shell}" == "bash" ]]; then
    [[ -f /usr/share/doc/fzf/examples/key-bindings.bash ]] && source /usr/share/doc/fzf/examples/key-bindings.bash
    [[ -f /usr/share/doc/fzf/examples/completion.bash ]] && source /usr/share/doc/fzf/examples/completion.bash
  elif [[ "${shine_ubuntu_sys_shell}" == "zsh" ]]; then
    [[ -f /usr/share/doc/fzf/examples/key-bindings.zsh ]] && source /usr/share/doc/fzf/examples/key-bindings.zsh
    [[ -f /usr/share/doc/fzf/examples/completion.zsh ]] && source /usr/share/doc/fzf/examples/completion.zsh
  fi
fi

# Atuin
if command -v atuin >/dev/null 2>&1; then
  eval "$(atuin init "${shine_ubuntu_sys_shell}")"
fi

# Starship prompt
if command -v starship >/dev/null 2>&1; then
  eval "$(starship init "${shine_ubuntu_sys_shell}")"
fi

# zoxide
if command -v zoxide >/dev/null 2>&1; then
  eval "$(zoxide init "${shine_ubuntu_sys_shell}")"
fi

# mise
if command -v mise >/dev/null 2>&1; then
  eval "$(mise activate "${shine_ubuntu_sys_shell}")"
elif [[ -x "$HOME/.local/bin/mise" ]]; then
  eval "$("$HOME/.local/bin/mise" activate "${shine_ubuntu_sys_shell}")"
fi

# zsh-vi-mode
if [[ "${shine_ubuntu_sys_shell}" == "zsh" ]]; then
  if [[ -f "/home/linuxbrew/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
    source "/home/linuxbrew/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
  elif [[ -f "$HOME/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
    source "$HOME/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
  elif [[ -f "$HOME/.local/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
    source "$HOME/.local/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
  fi
fi

if fzf --"${shine_ubuntu_sys_shell}" >/dev/null 2>&1; then
  eval "$(fzf --"${shine_ubuntu_sys_shell}")"
elif [[ "${shine_ubuntu_sys_shell}" == "bash" ]]; then
  [[ -f /usr/share/doc/fzf/examples/key-bindings.bash ]] && source /usr/share/doc/fzf/examples/key-bindings.bash
  [[ -f /usr/share/doc/fzf/examples/completion.bash ]] && source /usr/share/doc/fzf/examples/completion.bash
elif [[ "${shine_ubuntu_sys_shell}" == "zsh" ]]; then
  [[ -f /usr/share/doc/fzf/examples/key-bindings.zsh ]] && source /usr/share/doc/fzf/examples/key-bindings.zsh
  [[ -f /usr/share/doc/fzf/examples/completion.zsh ]] && source /usr/share/doc/fzf/examples/completion.zsh
fi

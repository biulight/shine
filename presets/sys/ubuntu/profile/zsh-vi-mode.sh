if [[ -f "/home/linuxbrew/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
  source "/home/linuxbrew/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
elif [[ -f "$HOME/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
  source "$HOME/.linuxbrew/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
elif [[ -f "$HOME/.local/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
  source "$HOME/.local/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
fi

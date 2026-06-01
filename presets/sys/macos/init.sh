#!/bin/zsh
# Initialize macOS with selectable Homebrew, terminal tools, editor, network, and JavaScript runtime setup steps.
emulate -L zsh
set -eu
set -o pipefail

ZSHRC_SENTINEL_START="# >>> shine macos sys >>>"
ZSHRC_SENTINEL_END="# <<< shine macos sys <<<"

ensure_macos() {
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "This sys init preset only supports macOS." >&2
        return 1
    fi
}

brew_executable() {
    if [[ -x /opt/homebrew/bin/brew ]]; then
        echo "/opt/homebrew/bin/brew"
        return
    fi
    if [[ -x /usr/local/bin/brew ]]; then
        echo "/usr/local/bin/brew"
        return
    fi
    return 1
}

load_homebrew_env() {
    local brew_path
    if brew_path=$(brew_executable); then
        eval "$("$brew_path" shellenv)"
    fi
}

install_homebrew() {
    load_homebrew_env
    if command -v brew &>/dev/null; then
        echo "Homebrew: already installed ($(brew --version | head -1))."
        return
    fi

    echo "Installing Homebrew..."
    NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    load_homebrew_env

    if ! command -v brew &>/dev/null; then
        echo "Homebrew installed, but brew is not available in this shell." >&2
        echo "Add Homebrew shellenv to your shell profile, then rerun this command." >&2
        return 1
    fi

    echo "Homebrew installed ($(brew --version | head -1))."
}

ensure_homebrew() {
    install_homebrew
}

brew_install_formula() {
    local formula="$1"
    local command_name="${2:-$formula}"

    ensure_homebrew
    if command -v "$command_name" &>/dev/null; then
        echo "$formula: already installed ($($command_name --version 2>/dev/null | head -1 || true))."
        return
    fi
    if brew list --formula "$formula" &>/dev/null; then
        echo "$formula: already installed with Homebrew."
        return
    fi

    echo "Installing $formula..."
    brew install "$formula"
}

remove_zshrc_block() {
    local file="$1"
    local tmp_file

    tmp_file="$(mktemp)"
    awk -v start="$ZSHRC_SENTINEL_START" -v end="$ZSHRC_SENTINEL_END" '
        $0 == start { skip = 1; next }
        $0 == end { skip = 0; next }
        !skip { print }
    ' "$file" > "$tmp_file"
    mv "$tmp_file" "$file"
}

append_zshrc_block() {
    local zshrc="$HOME/.zshrc"
    local block_file
    local added=0

    touch "$zshrc"
    remove_zshrc_block "$zshrc"
    block_file="$(mktemp)"

    if ! grep -Fq "HOMEBREW_PREFIX" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# Homebrew prefix cache
if [[ -d "/opt/homebrew" ]]; then
  export HOMEBREW_PREFIX="/opt/homebrew"
elif [[ -x "/usr/local/bin/brew" ]]; then
  export HOMEBREW_PREFIX="/usr/local"
else
  export HOMEBREW_PREFIX=""
fi

EOF
        added=1
    fi

    if ! grep -Fq "typeset -U path PATH" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# Basic PATH
typeset -U path PATH
path=(
  "$HOME/bin"
  "$HOME/.local/bin"
  "/usr/local/bin"
  "/usr/local/sbin"
  "/opt/homebrew/bin"
  "/opt/homebrew/sbin"
  $path
)
export PATH

EOF
        added=1
    fi

    if ! grep -Fq "NVM_DIR" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# nvm lazy load
export NVM_DIR="$HOME/.nvm"
nvm() {
  unfunction nvm 2>/dev/null

  if [[ -n "$HOMEBREW_PREFIX" && -s "$HOMEBREW_PREFIX/opt/nvm/nvm.sh" ]]; then
    source "$HOMEBREW_PREFIX/opt/nvm/nvm.sh"
  elif [[ -s "$NVM_DIR/nvm.sh" ]]; then
    source "$NVM_DIR/nvm.sh"
  fi

  nvm "$@"
}

EOF
        added=1
    fi

    if ! grep -Fq "BUN_INSTALL" "$zshrc" && ! grep -Fq '.bun' "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# Bun
export BUN_INSTALL="$HOME/.bun"
if [[ -d "$BUN_INSTALL/bin" ]]; then
  path=("$BUN_INSTALL/bin" $path)
fi

EOF
        added=1
    fi

    if ! grep -Fq "PNPM_HOME" "$zshrc" && ! grep -Fq "Library/pnpm" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# pnpm
export PNPM_HOME="$HOME/Library/pnpm"
if [[ -d "$PNPM_HOME" ]]; then
  case ":$PATH:" in
    *":$PNPM_HOME:"*) ;;
    *) path=("$PNPM_HOME" $path) ;;
  esac
fi
if [[ -d "$PNPM_HOME/bin" ]]; then
  case ":$PATH:" in
    *":$PNPM_HOME/bin:"*) ;;
    *) path=("$PNPM_HOME/bin" $path) ;;
  esac
fi

EOF
        added=1
    fi

    if ! grep -Fq "alias ls='eza" "$zshrc" && ! grep -Fq 'alias ls="eza' "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# eza
if command -v eza >/dev/null 2>&1; then
  alias ls='eza --icons'
  alias ll='eza -la --icons'
  alias lt='eza --tree'
fi

EOF
        added=1
    fi

    if ! grep -Fq "alias cat='bat" "$zshrc" && ! grep -Fq 'alias cat="bat' "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# bat
if command -v bat >/dev/null 2>&1; then
  alias cat='bat'
fi

EOF
        added=1
    fi

    if ! grep -Fq "yazi --cwd-file" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# Yazi
if command -v yazi >/dev/null 2>&1; then
  y() {
    local tmp cwd
    tmp="$(mktemp -t "yazi-cwd.XXXXXX")"
    command yazi "$@" --cwd-file="$tmp"
    IFS= read -r -d '' cwd < "$tmp"
    [[ "$cwd" != "$PWD" && -d "$cwd" ]] && builtin cd -- "$cwd"
    command rm -f -- "$tmp"
  }
fi

EOF
        added=1
    fi

    if ! grep -Fq "fzf --zsh" "$zshrc" && ! grep -Fq ".fzf.zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# fzf
if command -v fzf >/dev/null 2>&1; then
  eval "$(fzf --zsh)"
fi

EOF
        added=1
    fi

    if ! grep -Fq "atuin init zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# atuin
if command -v atuin >/dev/null 2>&1; then
  eval "$(atuin init zsh)"
fi

EOF
        added=1
    fi

    if ! grep -Fq "zoxide init zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# zoxide
if command -v zoxide >/dev/null 2>&1; then
  eval "$(zoxide init zsh)"
fi

EOF
        added=1
    fi

    if ! grep -Fq "zsh-vi-mode.plugin.zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# zsh-vi-mode
if [[ -n "$HOMEBREW_PREFIX" && -f "$HOMEBREW_PREFIX/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
  source "$HOMEBREW_PREFIX/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"
fi

EOF
        added=1
    fi

    if ! grep -Fq "fastfetch" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# fastfetch
# fastfetch can noticeably slow terminal startup, so run it manually when needed.
# if [[ -z "$ZELLIJ" ]] && command -v fastfetch >/dev/null 2>&1; then
#   fastfetch
# fi

EOF
        added=1
    fi

    if ! grep -Fq "starship init zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# Starship prompt
if command -v starship >/dev/null 2>&1; then
  eval "$(starship init zsh)"
fi

EOF
        added=1
    fi

    if ! grep -Fq "zsh-autosuggestions.zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# zsh-autosuggestions
if [[ -n "$HOMEBREW_PREFIX" && -f "$HOMEBREW_PREFIX/share/zsh-autosuggestions/zsh-autosuggestions.zsh" ]]; then
  source "$HOMEBREW_PREFIX/share/zsh-autosuggestions/zsh-autosuggestions.zsh"
fi

EOF
        added=1
    fi

    if ! grep -Fq "zsh-syntax-highlighting.zsh" "$zshrc"; then
        cat >> "$block_file" <<'EOF'
# zsh-syntax-highlighting must be near the end of .zshrc.
if [[ -n "$HOMEBREW_PREFIX" && -f "$HOMEBREW_PREFIX/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh" ]]; then
  source "$HOMEBREW_PREFIX/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"
fi

EOF
        added=1
    fi

    if [[ "$added" -eq 0 ]]; then
        rm -f "$block_file"
        echo "~/.zshrc already contains macOS shell tool initialization; no managed block needed."
        return
    fi

    {
        echo
        echo "$ZSHRC_SENTINEL_START"
        echo '# Managed by `shine sys init` for macOS. Existing user config is left untouched.'
        echo
        cat "$block_file"
        echo "$ZSHRC_SENTINEL_END"
    } >> "$zshrc"
    rm -f "$block_file"

    echo "Updated ~/.zshrc managed block for missing macOS shell tool initialization."
}

install_shell_formula() {
    brew_install_formula "$1" "${2:-$1}"
    append_zshrc_block
}

install_yazi() {
    brew_install_formula yazi yazi
}

install_starship() {
    install_shell_formula starship starship
}

install_neovim() {
    brew_install_formula neovim nvim
}

install_astronvim() {
    install_neovim
    brew_install_formula git git

    if [[ -d "$HOME/.config/nvim" ]]; then
        echo "AstroNvim: ~/.config/nvim already exists, skipping."
        return
    fi

    echo "Installing AstroNvim..."
    mkdir -p "$HOME/.config"
    git clone --depth 1 https://github.com/AstroNvim/template "$HOME/.config/nvim"
    rm -rf "$HOME/.config/nvim/.git"
    echo "AstroNvim installed. Run 'nvim' to finish plugin setup."
}

install_zerotier() {
    ensure_homebrew
    if command -v zerotier-cli &>/dev/null || [[ -d /Applications/ZeroTier.app ]]; then
        echo "ZeroTier: already installed."
    else
        echo "Installing ZeroTier One..."
        brew install --cask zerotier-one
    fi

    echo "ZeroTier next steps:"
    echo "  1. Open ZeroTier from Applications if the service is not running."
    echo "  2. Join your network: sudo zerotier-cli join <NETWORK_ID>"
    echo "  3. Approve the member in ZeroTier Central."
}

install_nvm() {
    install_shell_formula nvm nvm
    mkdir -p "$HOME/.nvm"

    echo "nvm shell setup, if not already configured:"
    echo "  export NVM_DIR=\"$HOME/.nvm\""
    echo "  [ -s \"$(brew --prefix nvm)/nvm.sh\" ] && . \"$(brew --prefix nvm)/nvm.sh\""
}

install_bun() {
    install_shell_formula bun bun
}

install_pnpm() {
    install_shell_formula pnpm pnpm
}

install_zsh_autosuggestions() {
    install_shell_formula zsh-autosuggestions zsh-autosuggestions
}

install_zsh_syntax_highlighting() {
    install_shell_formula zsh-syntax-highlighting zsh-syntax-highlighting
}

install_zsh_vi_mode() {
    install_shell_formula zsh-vi-mode zsh-vi-mode
}

install_zoxide() {
    install_shell_formula zoxide zoxide
}

install_atuin() {
    install_shell_formula atuin atuin
}

install_fzf() {
    install_shell_formula fzf fzf
}

install_bat() {
    install_shell_formula bat bat
}

install_eza() {
    install_shell_formula eza eza
}

install_fastfetch() {
    brew_install_formula fastfetch fastfetch
}

run_item() {
    case "${1:-}" in
        homebrew) install_homebrew ;;
        yazi) install_yazi ;;
        starship) install_starship ;;
        neovim) install_neovim ;;
        astronvim) install_astronvim ;;
        zerotier) install_zerotier ;;
        zsh-autosuggestions) install_zsh_autosuggestions ;;
        zsh-syntax-highlighting) install_zsh_syntax_highlighting ;;
        zsh-vi-mode) install_zsh_vi_mode ;;
        zoxide) install_zoxide ;;
        atuin) install_atuin ;;
        fzf) install_fzf ;;
        bat) install_bat ;;
        eza) install_eza ;;
        nvm) install_nvm ;;
        bun) install_bun ;;
        pnpm) install_pnpm ;;
        fastfetch) install_fastfetch ;;
        "") return 0 ;;
        *)
            echo "Unknown sys init item: $1" >&2
            return 1
            ;;
    esac
}

ensure_macos

for item in "$@"; do
    run_item "$item"
done

if [[ $# -gt 0 ]]; then
    echo "Done."
fi

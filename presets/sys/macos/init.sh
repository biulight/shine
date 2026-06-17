#!/bin/zsh
# Initialize macOS with selectable Homebrew, Rust, terminal tools, editor, network, and JavaScript runtime setup steps.
emulate -L zsh
set -eu
set -o pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"

status() {
    local state="$1"
    local detail="${2:-}"
    printf 'SHINE_SYS_STATUS\t%s\t%s\n' "$state" "$detail"
}

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
        status "already-installed" "$(brew --version | head -1)"
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

    status "installed" "$(brew --version | head -1)"
}

ensure_homebrew() {
    install_homebrew
}

brew_install_formula() {
    local formula="$1"
    local command_name="${2:-$formula}"

    ensure_homebrew
    if command -v "$command_name" &>/dev/null; then
        status "already-installed" "$($command_name --version 2>/dev/null | head -1 || true)"
        return
    fi
    if brew list --formula "$formula" &>/dev/null; then
        status "already-installed" "Homebrew formula"
        return
    fi

    echo "Installing $formula..."
    brew install "$formula"
    status "installed" "$formula"
}

install_shell_formula() {
    brew_install_formula "$1" "${2:-$1}"
}

install_rust() {
    if command -v rustup &>/dev/null; then
        status "already-installed" "$(rustup --version | head -1)"
        return
    fi

    if [[ -x "$HOME/.cargo/bin/rustup" ]]; then
        status "already-installed" "$("$HOME/.cargo/bin/rustup" --version | head -1)"
        return
    fi

    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    export PATH="$HOME/.cargo/bin:$PATH"
    status "installed" "$(rustc --version)"
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
        status "skipped" "~/.config/nvim already exists"
        return
    fi

    echo "Installing AstroNvim..."
    mkdir -p "$HOME/.config"
    git clone --depth 1 https://github.com/AstroNvim/template "$HOME/.config/nvim"
    rm -rf "$HOME/.config/nvim/.git"
    status "installed" "~/.config/nvim"
}

install_zerotier() {
    ensure_homebrew
    if command -v zerotier-cli &>/dev/null || [[ -d /Applications/ZeroTier.app ]]; then
        status "already-installed"
    else
        echo "Installing ZeroTier One..."
        brew install --cask zerotier-one
        status "needs-action" "open ZeroTier and join a network"
    fi
}

install_nvm() {
    install_shell_formula nvm nvm
    mkdir -p "$HOME/.nvm"
}

install_bun() {
    install_shell_formula bun bun
}

install_pnpm() {
    install_shell_formula pnpm pnpm
    mkdir -p "$HOME/Library/pnpm/bin"
}

install_mise() {
    brew_install_formula mise mise
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
        rust) install_rust ;;
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
        mise) install_mise ;;
        fastfetch) install_fastfetch ;;
        __shine_finalize) status "completed" "profile is managed by shine CLI" ;;
        "") return 0 ;;
        *)
            echo "Unknown sys init item: $1" >&2
            return 1
            ;;
    esac
}

ensure_macos

run_item "${1:-}"

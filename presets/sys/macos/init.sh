#!/bin/zsh
# Initialize macOS with selectable Homebrew, terminal tools, editor, network, and JavaScript runtime setup steps.
emulate -L zsh
set -eu
set -o pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
ZSHRC_SENTINEL_START="# >>> shine macos sys >>>"
ZSHRC_SENTINEL_END="# <<< shine macos sys <<<"

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

remove_zshrc_block() {
    local file="$1"
    local tmp_file

    [[ -f "$file" ]] || return 0
    tmp_file="$(mktemp)"
    awk -v start="$ZSHRC_SENTINEL_START" -v end="$ZSHRC_SENTINEL_END" '
        $0 == start { skip = 1; next }
        $0 == end { skip = 0; next }
        !skip { print }
    ' "$file" > "$tmp_file"
    mv "$tmp_file" "$file"
}

managed_profile_path() {
    echo "$HOME/.shine/profile/macos-sys.sh"
}

install_managed_profile_script() {
    local template_path="$SCRIPT_DIR/profile.sh"
    local managed_path
    local managed_parent
    local updated=0

    if [[ ! -f "$template_path" ]]; then
        echo "Missing macOS profile template: $template_path" >&2
        return 2
    fi

    managed_path="$(managed_profile_path)"
    managed_parent="$(dirname "$managed_path")"
    mkdir -p "$managed_parent"
    if [[ ! -f "$managed_path" ]] || ! cmp -s "$template_path" "$managed_path"; then
        cp "$template_path" "$managed_path"
        updated=1
    fi
    return "$updated"
}

append_zshrc_block() {
    local zshrc="$HOME/.zshrc"
    local current_block
    local desired_block

    touch "$zshrc"
    current_block="$(mktemp)"
    desired_block="$(mktemp)"

    awk -v start="$ZSHRC_SENTINEL_START" -v end="$ZSHRC_SENTINEL_END" '
        $0 == start { capture = 1 }
        capture { print }
        $0 == end { capture = 0 }
    ' "$zshrc" > "$current_block"

    {
        echo "$ZSHRC_SENTINEL_START"
        echo 'shine_macos_sys_profile="$HOME/.shine/profile/macos-sys.sh"'
        echo 'if [[ -f "$shine_macos_sys_profile" ]]; then'
        echo '  source "$shine_macos_sys_profile"'
        echo 'fi'
        echo "$ZSHRC_SENTINEL_END"
    } > "$desired_block"

    if cmp -s "$desired_block" "$current_block"; then
        rm -f "$current_block" "$desired_block"
        return 1
    fi

    remove_zshrc_block "$zshrc"
    {
        echo
        cat "$desired_block"
    } >> "$zshrc"

    rm -f "$current_block" "$desired_block"
    return 0
}

append_zshrc_init_block() {
    local managed_path
    local profile_updated=0
    local block_updated=0

    if install_managed_profile_script; then
        profile_updated=0
    else
        case "$?" in
            1) profile_updated=1 ;;
            *) return 1 ;;
        esac
    fi

    if append_zshrc_block; then
        block_updated=1
    fi

    managed_path="$(managed_profile_path)"
    if [[ "$profile_updated" -eq 1 || "$block_updated" -eq 1 ]]; then
        status "updated" "~/.zshrc -> $managed_path"
    else
        status "skipped" "~/.zshrc already configured"
    fi
}

install_shell_formula() {
    brew_install_formula "$1" "${2:-$1}"
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
        __shine_finalize) append_zshrc_init_block ;;
        "") return 0 ;;
        *)
            echo "Unknown sys init item: $1" >&2
            return 1
            ;;
    esac
}

ensure_macos

run_item "${1:-}"

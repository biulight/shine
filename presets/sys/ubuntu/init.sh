#!/bin/bash
# Initialize Ubuntu system with selectable Neovim, AstroNvim, Atuin, Yazi, Starship, zoxide, zsh-vi-mode, fzf, bat, eza, pnpm, mise, Homebrew, and ZeroTier steps.
set -euo pipefail

ARCH=$(uname -m)
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SHELL_SENTINEL_START="# >>> shine ubuntu sys >>>"
SHELL_SENTINEL_END="# <<< shine ubuntu sys <<<"
PNPM_HOME="${PNPM_HOME:-$HOME/.local/share/pnpm}"
ZSH_VI_MODE_PLUGIN="$HOME/.local/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh"

export PATH="$HOME/.local/bin:$PNPM_HOME:$PNPM_HOME/bin:$PATH"

status() {
    local state="$1"
    local detail="${2:-}"
    printf 'SHINE_SYS_STATUS\t%s\t%s\n' "$state" "$detail"
}

brew_executable() {
    if [[ -x /home/linuxbrew/.linuxbrew/bin/brew ]]; then
        echo "/home/linuxbrew/.linuxbrew/bin/brew"
        return
    fi
    if [[ -x "$HOME/.linuxbrew/bin/brew" ]]; then
        echo "$HOME/.linuxbrew/bin/brew"
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

load_atuin_env() {
    if [[ -f "$HOME/.atuin/bin/env" ]]; then
        . "$HOME/.atuin/bin/env"
    fi
}

remove_shell_block() {
    local file="$1"
    local tmp_file

    [[ -f "$file" ]] || return 0
    tmp_file="$(mktemp)"
    awk -v start="$SHELL_SENTINEL_START" -v end="$SHELL_SENTINEL_END" '
        $0 == start { skip = 1; next }
        $0 == end { skip = 0; next }
        !skip { print }
    ' "$file" > "$tmp_file"
    mv "$tmp_file" "$file"
}

remove_pnpm_block() {
    local file="$1"
    local tmp_file

    [[ -f "$file" ]] || return 0
    tmp_file="$(mktemp)"
    awk '
        $0 == "# pnpm" { skip = 1; next }
        $0 == "# pnpm end" { skip = 0; next }
        !skip { print }
    ' "$file" > "$tmp_file"
    mv "$tmp_file" "$file"
}

append_shell_block() {
    local file="$1"
    local shell_name="$2"

    touch "$file"
    remove_shell_block "$file"
    remove_pnpm_block "$file"

    {
        echo
        echo "$SHELL_SENTINEL_START"
        echo "shine_ubuntu_sys_profile=\"\$HOME/.shine/profile/ubuntu-sys.sh\""
        echo "if [[ -f \"\$shine_ubuntu_sys_profile\" ]]; then"
        echo "  SHINE_UBUNTU_SYS_SHELL=\"$shell_name\""
        echo "  source \"\$shine_ubuntu_sys_profile\""
        echo "fi"
        echo "$SHELL_SENTINEL_END"
    } >> "$file"
}

managed_profile_path() {
    echo "$HOME/.shine/profile/ubuntu-sys.sh"
}

install_managed_profile_script() {
    local template_path="$SCRIPT_DIR/profile.sh"
    local managed_path
    local managed_parent

    if [[ ! -f "$template_path" ]]; then
        echo "Missing Ubuntu profile template: $template_path" >&2
        return 1
    fi

    managed_path="$(managed_profile_path)"
    managed_parent="$(dirname "$managed_path")"
    mkdir -p "$managed_parent"
    cp "$template_path" "$managed_path"
}

append_shell_init_blocks() {
    local sys_shell="${SHINE_SYS_SHELL:-bash}"
    local managed_path

    install_managed_profile_script
    managed_path="$(managed_profile_path)"
    case "$sys_shell" in
        bash)
            append_shell_block "$HOME/.bashrc" bash
            remove_shell_block "$HOME/.zshrc"
            status "updated" "~/.bashrc -> $managed_path"
            ;;
        zsh)
            append_shell_block "$HOME/.zshrc" zsh
            remove_shell_block "$HOME/.bashrc"
            status "updated" "~/.zshrc -> $managed_path"
            ;;
        *)
            status "skipped" "unsupported shell: $sys_shell"
            ;;
    esac
}

install_packages() {
    local packages=()
    local package

    for package in "$@"; do
        if ! dpkg -s "$package" &>/dev/null; then
            packages+=("$package")
        fi
    done

    if [[ ${#packages[@]} -eq 0 ]]; then
        return
    fi

    echo "Installing packages: ${packages[*]}"
    sudo apt-get update
    sudo apt-get install -y "${packages[@]}"
}

apt_package_available() {
    local package="$1"
    apt-cache show "$package" &>/dev/null
}

# --- Neovim ---

neovim_version_ok() {
    command -v nvim &>/dev/null || return 1
    local minor
    minor=$(nvim --version | head -1 | sed 's/NVIM v[0-9]*\.\([0-9]*\).*/\1/')
    [[ "$minor" -ge 10 ]]
}

install_neovim() {
    if neovim_version_ok; then
        status "already-installed" "$(nvim --version | head -1)"
        return
    fi
    echo "Installing Neovim (latest stable)..."
    local tarball
    case "$ARCH" in
        x86_64)  tarball="nvim-linux-x86_64.tar.gz" ;;
        aarch64) tarball="nvim-linux-arm64.tar.gz" ;;
        *) echo "Unsupported arch: $ARCH" >&2; return 1 ;;
    esac
    local stem="${tarball%.tar.gz}"
    curl -fsSL "https://github.com/neovim/neovim/releases/latest/download/${tarball}" \
        -o /tmp/nvim.tar.gz
    sudo tar xzf /tmp/nvim.tar.gz -C /opt
    sudo ln -sf "/opt/${stem}/bin/nvim" /usr/local/bin/nvim
    rm /tmp/nvim.tar.gz
    status "installed" "/usr/local/bin/nvim"
}

# --- AstroNvim ---

install_astronvim() {
    if [[ -d "$HOME/.config/nvim" ]]; then
        status "skipped" "~/.config/nvim already exists"
        return
    fi
    echo "Installing AstroNvim..."
    sudo apt-get install -y git
    git clone --depth 1 https://github.com/AstroNvim/template "$HOME/.config/nvim"
    rm -rf "$HOME/.config/nvim/.git"
    status "installed" "~/.config/nvim"
}

# --- Atuin ---

install_atuin() {
    load_atuin_env
    if command -v atuin &>/dev/null; then
        status "already-installed" "$(atuin --version)"
        return
    fi
    echo "Installing Atuin..."
    curl --proto '=https' --tlsv1.2 -LsSf https://setup.atuin.sh | sh
    load_atuin_env
    status "installed" "$(atuin --version)"
}

# --- Yazi ---

install_yazi_dependencies() {
    install_packages file ffmpeg 7zip jq poppler-utils fd-find ripgrep fzf imagemagick xclip
}

ensure_fd_alias() {
    if command -v fd &>/dev/null; then
        return
    fi
    if ! command -v fdfind &>/dev/null; then
        return
    fi

    echo "Creating fd -> fdfind symlink..."
    sudo ln -sf "$(command -v fdfind)" /usr/local/bin/fd
}

install_yazi() {
    install_yazi_dependencies

    if command -v yazi &>/dev/null; then
        ensure_fd_alias
        status "already-installed" "$(yazi --version | head -1)"
        return
    fi

    local target
    case "$ARCH" in
        x86_64)  target="x86_64-unknown-linux-gnu" ;;
        aarch64) target="aarch64-unknown-linux-gnu" ;;
        *) echo "Unsupported arch for Yazi: $ARCH" >&2; return 1 ;;
    esac

    echo "Installing Yazi from the latest official release..."
    local version package tmp
    version=$(curl -fsSL -o /dev/null -w '%{url_effective}' https://github.com/sxyazi/yazi/releases/latest | sed 's#.*/tag/v##')
    package="yazi-${target}.deb"
    tmp="/tmp/${package}"
    curl -fsSL "https://github.com/sxyazi/yazi/releases/download/v${version}/${package}" -o "$tmp"
    sudo apt-get install -y "$tmp"
    rm -f "$tmp"

    ensure_fd_alias
    status "installed" "$(yazi --version | head -1)"
}

# --- Starship ---

install_starship() {
    if command -v starship &>/dev/null; then
        status "already-installed" "$(starship --version | head -1)"
        return
    fi

    echo "Installing Starship..."
    curl -sS https://starship.rs/install.sh | sudo sh -s -- -y -b /usr/local/bin
    status "installed" "$(starship --version | head -1)"
}

# --- zoxide ---

install_zoxide() {
    if command -v zoxide &>/dev/null; then
        status "already-installed" "$(zoxide --version | head -1)"
        return
    fi

    if [[ -x "$HOME/.local/bin/zoxide" ]]; then
        status "already-installed" "$("$HOME/.local/bin/zoxide" --version | head -1)"
        return
    fi

    echo "Installing zoxide..."
    mkdir -p "$HOME/.local/bin"
    curl -sSfL https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/install.sh | sh
    status "installed" "$(zoxide --version | head -1)"
}

# --- zsh-vi-mode ---

install_zsh_vi_mode() {
    load_homebrew_env
    if [[ -n "${HOMEBREW_PREFIX:-}" ]] && [[ -f "$HOMEBREW_PREFIX/opt/zsh-vi-mode/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh" ]]; then
        status "already-installed" "Homebrew"
        return
    fi

    if command -v brew &>/dev/null; then
        echo "Installing zsh-vi-mode with Homebrew..."
        brew install zsh-vi-mode
        status "installed" "Homebrew"
        return
    fi

    if [[ -f "$ZSH_VI_MODE_PLUGIN" ]]; then
        status "already-installed" "$ZSH_VI_MODE_PLUGIN"
        return
    fi

    install_packages git
    echo "Installing zsh-vi-mode to $ZSH_VI_MODE_PLUGIN..."
    mkdir -p "$(dirname "$ZSH_VI_MODE_PLUGIN")"
    git clone --depth 1 https://github.com/jeffreytse/zsh-vi-mode.git "$(dirname "$ZSH_VI_MODE_PLUGIN")"
    status "installed" "$ZSH_VI_MODE_PLUGIN"
}

# --- fzf ---

install_fzf() {
    if command -v fzf &>/dev/null; then
        status "already-installed" "$(fzf --version | head -1)"
        return
    fi

    install_packages fzf
    status "installed" "$(fzf --version | head -1)"
}

# --- bat ---

ensure_bat_command() {
    if command -v bat &>/dev/null; then
        return
    fi
    if ! command -v batcat &>/dev/null; then
        return
    fi

    echo "Creating bat -> batcat symlink..."
    sudo ln -sf "$(command -v batcat)" /usr/local/bin/bat
}

install_bat() {
    if command -v bat &>/dev/null; then
        status "already-installed" "$(bat --version | head -1)"
        return
    fi

    install_packages bat
    ensure_bat_command

    if command -v bat &>/dev/null; then
        status "installed" "$(bat --version | head -1)"
    else
        echo "bat package installed, but no bat or batcat command was found." >&2
        return 1
    fi
}

# --- eza ---

install_eza() {
    load_homebrew_env
    if command -v eza &>/dev/null; then
        status "already-installed" "$(eza --version | head -1)"
        return
    fi

    if apt_package_available eza; then
        install_packages eza
    else
        install_homebrew
        echo "Installing eza with Homebrew..."
        brew install eza
    fi

    status "installed" "$(eza --version | head -1)"
}

# --- pnpm ---

install_pnpm() {
    install_packages libatomic1

    if command -v pnpm &>/dev/null; then
        status "already-installed" "$(pnpm --version)"
        return
    fi

    if [[ -x "$HOME/.local/share/pnpm/pnpm" ]]; then
        status "already-installed" "$("$HOME/.local/share/pnpm/pnpm" --version)"
        return
    fi

    echo "Installing pnpm..."
    curl -fsSL https://get.pnpm.io/install.sh | SHELL="$(command -v bash)" sh -
    status "installed" "$(pnpm --version)"
}

# --- mise ---

install_mise() {
    if command -v mise &>/dev/null; then
        status "already-installed" "$(mise --version | head -1)"
        return
    fi

    if [[ -x "$HOME/.local/bin/mise" ]]; then
        status "already-installed" "$("$HOME/.local/bin/mise" --version | head -1)"
        return
    fi

    echo "Installing mise..."
    mkdir -p "$HOME/.local/bin"
    curl -fsSL https://mise.run | sh
    status "installed" "$("$HOME/.local/bin/mise" --version | head -1)"
}

# --- Homebrew ---

install_homebrew() {
    load_homebrew_env
    if command -v brew &>/dev/null; then
        status "already-installed" "$(brew --version | head -1)"
        return
    fi

    if [[ "$(id -u)" -eq 0 ]]; then
        echo "Homebrew cannot be installed as root. Run this item as a non-root user or skip Homebrew-dependent items." >&2
        return 1
    fi

    install_packages build-essential procps curl file git

    echo "Installing Homebrew..."
    NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    load_homebrew_env

    if ! command -v brew &>/dev/null; then
        echo "Homebrew installed, but brew is not available in this shell." >&2
        echo "Open a new shell or source ~/.bashrc / ~/.zshrc, then rerun this command." >&2
        return 1
    fi

    status "installed" "$(brew --version | head -1)"
}

# --- ZeroTier ---

install_zerotier() {
    if command -v zerotier-cli &>/dev/null || command -v zerotier-one &>/dev/null; then
        if command -v zerotier-cli &>/dev/null; then
            status "already-installed" "$(zerotier-cli -v)"
        else
            status "already-installed"
        fi
        return
    fi

    echo "Installing ZeroTier..."
    curl -s https://install.zerotier.com | sudo bash

    status "needs-action" "join a network with: sudo zerotier-cli join <NETWORK_ID>"
}

run_item() {
    case "${1:-}" in
        neovim) install_neovim ;;
        astronvim) install_astronvim ;;
        atuin) install_atuin ;;
        yazi) install_yazi ;;
        starship) install_starship ;;
        zoxide) install_zoxide ;;
        zsh-vi-mode) install_zsh_vi_mode ;;
        fzf) install_fzf ;;
        bat) install_bat ;;
        eza) install_eza ;;
        pnpm) install_pnpm ;;
        mise) install_mise ;;
        homebrew) install_homebrew ;;
        zerotier) install_zerotier ;;
        __shine_finalize) append_shell_init_blocks ;;
        "") return 0 ;;
        *)
            echo "Unknown sys init item: $1" >&2
            return 1
            ;;
    esac
}

run_item "${1:-}"

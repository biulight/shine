#!/bin/bash
# Initialize Ubuntu system: installs Neovim (v0.10+), AstroNvim, and Atuin.
set -euo pipefail

ARCH=$(uname -m)

# --- Neovim ---

neovim_version_ok() {
    command -v nvim &>/dev/null || return 1
    local minor
    minor=$(nvim --version | head -1 | sed 's/NVIM v[0-9]*\.\([0-9]*\).*/\1/')
    [[ "$minor" -ge 10 ]]
}

install_neovim() {
    if neovim_version_ok; then
        echo "Neovim: already installed ($(nvim --version | head -1))."
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
    echo "Neovim installed to /usr/local/bin/nvim."
}

# --- AstroNvim ---

install_astronvim() {
    if [[ -d "$HOME/.config/nvim" ]]; then
        echo "AstroNvim: ~/.config/nvim already exists, skipping."
        return
    fi
    echo "Installing AstroNvim..."
    sudo apt-get install -y git
    git clone --depth 1 https://github.com/AstroNvim/template "$HOME/.config/nvim"
    rm -rf "$HOME/.config/nvim/.git"
    echo "AstroNvim installed. Run 'nvim' to finish plugin setup."
}

# --- Atuin ---

install_atuin() {
    if command -v atuin &>/dev/null; then
        echo "Atuin: already installed ($(atuin --version))."
        return
    fi
    echo "Installing Atuin..."
    curl --proto '=https' --tlsv1.2 -LsSf https://setup.atuin.sh | sh
}

install_neovim
install_astronvim
install_atuin

echo "Done."

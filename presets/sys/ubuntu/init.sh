#!/bin/bash
# Initialize Ubuntu system: installs Neovim and Atuin.
set -euo pipefail

install_neovim() {
    if command -v nvim &>/dev/null; then
        echo "Neovim: already installed ($(nvim --version | head -1))."
        return
    fi
    echo "Installing Neovim..."
    sudo apt-get update -y
    sudo apt-get install -y neovim
}

install_atuin() {
    if command -v atuin &>/dev/null; then
        echo "Atuin: already installed ($(atuin --version))."
        return
    fi
    echo "Installing Atuin..."
    curl --proto '=https' --tlsv1.2 -LsSf https://setup.atuin.sh | sh
}

install_neovim
install_atuin

echo "Done."

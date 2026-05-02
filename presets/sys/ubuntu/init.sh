#!/bin/bash
# Initialize Ubuntu system: installs Atuin and Neovim.
set -euo pipefail

if command -v nvim &>/dev/null; then
    echo "Neovim: already installed."
else
    echo "Installing Neovim..."
    sudo apt-get update -y
    sudo apt-get install -y neovim
fi

if command -v atuin &>/dev/null; then
    echo "Atuin: already installed."
else
    echo "Installing Atuin..."
    bash <(curl --proto '=https' --tlsv1.2 -sSf https://setup.atuin.sh)
fi

echo "Done."

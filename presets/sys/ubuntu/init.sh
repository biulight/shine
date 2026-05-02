#!/bin/bash
# Initialize Ubuntu system: installs Neovim and Atuin.
set -euo pipefail

ARCH=$(uname -m)

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

    local target
    case "$ARCH" in
        x86_64)           target="x86_64-unknown-linux-musl" ;;
        aarch64 | arm64) target="aarch64-unknown-linux-musl" ;;
        *)
            echo "Unsupported architecture: $ARCH. Install Atuin manually." >&2
            return 1
            ;;
    esac

    echo "Installing Atuin (${target})..."
    local version
    version=$(curl -fsSL "https://api.github.com/repos/atuinsh/atuin/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"v\([^"]*\)".*/\1/')

    local url="https://github.com/atuinsh/atuin/releases/download/v${version}/atuin-${target}.tar.gz"
    echo "Downloading v${version}..."
    curl -fsSL "$url" -o /tmp/atuin.tar.gz
    tar xzf /tmp/atuin.tar.gz -C /tmp
    sudo install -m 0755 "/tmp/atuin-${target}/atuin" /usr/local/bin/atuin
    rm -rf /tmp/atuin.tar.gz "/tmp/atuin-${target}"
    echo "Atuin installed to /usr/local/bin/atuin."
}

install_neovim
install_atuin

echo "Done."

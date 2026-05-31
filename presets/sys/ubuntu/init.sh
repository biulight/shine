#!/bin/bash
# Initialize Ubuntu system with selectable Neovim, AstroNvim, Atuin, Yazi, Starship, zoxide, pnpm, mise, Homebrew, and ZeroTier steps.
set -euo pipefail

ARCH=$(uname -m)
SHELL_SENTINEL_START="# >>> shine ubuntu sys >>>"
SHELL_SENTINEL_END="# <<< shine ubuntu sys <<<"
PNPM_HOME="${PNPM_HOME:-$HOME/.local/share/pnpm}"

export PATH="$HOME/.local/bin:$PNPM_HOME:$PNPM_HOME/bin:$PATH"

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

remove_shell_block() {
    local file="$1"
    local tmp_file

    [[ -f "$file" ]] || return
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

    [[ -f "$file" ]] || return
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
    local init_file

    touch "$file"
    remove_shell_block "$file"
    remove_pnpm_block "$file"
    init_file="$(mktemp)"

    cat > "$init_file" <<EOF
# Managed by \`shine sys init\` for Ubuntu. Existing user config is left untouched.

# User-local binaries
case ":\$PATH:" in
  *":\$HOME/.local/bin:"*) ;;
  *) export PATH="\$HOME/.local/bin:\$PATH" ;;
esac

# Homebrew
if [[ -x "/home/linuxbrew/.linuxbrew/bin/brew" ]]; then
  eval "\$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"
elif [[ -x "\$HOME/.linuxbrew/bin/brew" ]]; then
  eval "\$(\$HOME/.linuxbrew/bin/brew shellenv)"
fi

# pnpm
export PNPM_HOME="\$HOME/.local/share/pnpm"
case ":\$PATH:" in
  *":\$PNPM_HOME:"*) ;;
  *) export PATH="\$PNPM_HOME:\$PATH" ;;
esac
if [[ -d "\$PNPM_HOME/bin" ]]; then
  case ":\$PATH:" in
    *":\$PNPM_HOME/bin:"*) ;;
    *) export PATH="\$PNPM_HOME/bin:\$PATH" ;;
  esac
fi

# Starship prompt
if command -v starship >/dev/null 2>&1; then
  eval "\$(starship init ${shell_name})"
fi

# zoxide
if command -v zoxide >/dev/null 2>&1; then
  eval "\$(zoxide init ${shell_name})"
fi

# mise
if command -v mise >/dev/null 2>&1; then
  eval "\$(mise activate ${shell_name})"
elif [[ -x "\$HOME/.local/bin/mise" ]]; then
  eval "\$(\$HOME/.local/bin/mise activate ${shell_name})"
fi
EOF

    {
        echo
        echo "$SHELL_SENTINEL_START"
        cat "$init_file"
        echo "$SHELL_SENTINEL_END"
    } >> "$file"
    rm -f "$init_file"
}

append_shell_init_blocks() {
    append_shell_block "$HOME/.bashrc" bash
    append_shell_block "$HOME/.zshrc" zsh
    echo "Updated ~/.bashrc and ~/.zshrc managed blocks for Ubuntu shell tool initialization."
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
        echo "Packages already installed: $*"
        return
    fi

    echo "Installing packages: ${packages[*]}"
    sudo apt-get update
    sudo apt-get install -y "${packages[@]}"
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
        echo "Yazi: already installed ($(yazi --version | head -1))."
        ensure_fd_alias
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
    echo "Yazi installed ($(yazi --version | head -1))."
}

# --- Starship ---

install_starship() {
    if command -v starship &>/dev/null; then
        echo "Starship: already installed ($(starship --version | head -1))."
        append_shell_init_blocks
        return
    fi

    local target
    case "$ARCH" in
        x86_64)  target="x86_64-unknown-linux-gnu" ;;
        aarch64) target="aarch64-unknown-linux-gnu" ;;
        *) echo "Unsupported arch for Starship: $ARCH" >&2; return 1 ;;
    esac

    echo "Installing Starship..."
    local tmp
    tmp="/tmp/starship-${target}.tar.gz"
    curl -fsSL "https://github.com/starship/starship/releases/latest/download/starship-${target}.tar.gz" -o "$tmp"
    sudo tar xzf "$tmp" -C /usr/local/bin starship
    sudo chmod 755 /usr/local/bin/starship
    rm -f "$tmp"
    append_shell_init_blocks
    echo "Starship installed ($(starship --version | head -1))."
}

# --- zoxide ---

install_zoxide() {
    if command -v zoxide &>/dev/null; then
        echo "zoxide: already installed ($(zoxide --version | head -1))."
        append_shell_init_blocks
        return
    fi

    if [[ -x "$HOME/.local/bin/zoxide" ]]; then
        echo "zoxide: already installed ($("$HOME/.local/bin/zoxide" --version | head -1))."
        append_shell_init_blocks
        return
    fi

    echo "Installing zoxide..."
    mkdir -p "$HOME/.local/bin"
    curl -sSfL https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/install.sh | sh
    append_shell_init_blocks
    echo "zoxide installed ($(zoxide --version | head -1))."
}

# --- pnpm ---

install_pnpm() {
    install_packages libatomic1

    if command -v pnpm &>/dev/null; then
        echo "pnpm: already installed ($(pnpm --version))."
        append_shell_init_blocks
        return
    fi

    if [[ -x "$HOME/.local/share/pnpm/pnpm" ]]; then
        echo "pnpm: already installed ($("$HOME/.local/share/pnpm/pnpm" --version))."
        append_shell_init_blocks
        return
    fi

    echo "Installing pnpm..."
    curl -fsSL https://get.pnpm.io/install.sh | SHELL="$(command -v bash)" sh -
    append_shell_init_blocks
    echo "pnpm installed ($(pnpm --version))."
}

# --- mise ---

install_mise() {
    if command -v mise &>/dev/null; then
        echo "mise: already installed ($(mise --version | head -1))."
        append_shell_init_blocks
        return
    fi

    if [[ -x "$HOME/.local/bin/mise" ]]; then
        echo "mise: already installed ($("$HOME/.local/bin/mise" --version | head -1))."
        append_shell_init_blocks
        return
    fi

    echo "Installing mise..."
    mkdir -p "$HOME/.local/bin"
    curl -fsSL https://mise.run | sh
    append_shell_init_blocks
    echo "mise installed ($("$HOME/.local/bin/mise" --version | head -1))."
}

# --- Homebrew ---

install_homebrew() {
    load_homebrew_env
    if command -v brew &>/dev/null; then
        echo "Homebrew: already installed ($(brew --version | head -1))."
        append_shell_init_blocks
        return
    fi

    install_packages build-essential procps curl file git

    echo "Installing Homebrew..."
    NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    load_homebrew_env
    append_shell_init_blocks

    if ! command -v brew &>/dev/null; then
        echo "Homebrew installed, but brew is not available in this shell." >&2
        echo "Open a new shell or source ~/.bashrc / ~/.zshrc, then rerun this command." >&2
        return 1
    fi

    echo "Homebrew installed ($(brew --version | head -1))."
}

# --- ZeroTier ---

install_zerotier() {
    if command -v zerotier-cli &>/dev/null || command -v zerotier-one &>/dev/null; then
        if command -v zerotier-cli &>/dev/null; then
            echo "ZeroTier: already installed ($(zerotier-cli -v))."
        else
            echo "ZeroTier: already installed."
        fi
        return
    fi

    echo "Installing ZeroTier..."
    curl -s https://install.zerotier.com | sudo bash

    echo "ZeroTier installed."
    echo "Next steps for custom planet/network setup:"
    echo "  1. Replace the planet file under /var/lib/zerotier-one."
    echo "  2. Restart the service: sudo service zerotier-one restart"
    echo "  3. Join your network: sudo zerotier-cli join <NETWORK_ID>"
    echo "  4. Approve the member in ZeroTier Central."
    echo "  5. Verify peers: sudo zerotier-cli peers"
    echo "     Look for a peer with role planet."
}

run_item() {
    case "${1:-}" in
        neovim) install_neovim ;;
        astronvim) install_astronvim ;;
        atuin) install_atuin ;;
        yazi) install_yazi ;;
        starship) install_starship ;;
        zoxide) install_zoxide ;;
        pnpm) install_pnpm ;;
        mise) install_mise ;;
        homebrew) install_homebrew ;;
        zerotier) install_zerotier ;;
        "") return 0 ;;
        *)
            echo "Unknown sys init item: $1" >&2
            return 1
            ;;
    esac
}

for item in "$@"; do
    run_item "$item"
done

if [[ $# -gt 0 ]]; then
    echo "Done."
fi

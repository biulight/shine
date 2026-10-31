#!/bin/sh

set -eu

SHINE_REPO="${SHINE_REPO:-biulight/shine}"
SHINE_INSTALL_DIR="${SHINE_INSTALL_DIR:-$HOME/.local/bin}"
SHINE_VERSION="${SHINE_VERSION:-latest}"

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os="darwin" ;;
    Linux) os="linux" ;;
    *) fail "unsupported operating system: $os" ;;
  esac

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) fail "unsupported architecture: $arch" ;;
  esac

  printf '%s-%s' "$os" "$arch"
}

build_download_url() {
  version="$1"
  target="$2"
  asset="shine-v${version}-${target}.tar.gz"
  printf 'https://github.com/%s/releases/download/v%s/%s' "$SHINE_REPO" "$version" "$asset"
}

download_file() {
  url="$1"
  dest="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
    return
  fi

  fail "either curl or wget is required"
}

resolve_version() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "https://api.github.com/repos/${SHINE_REPO}/releases/latest" \
      | grep '"tag_name"' \
      | sed 's/.*"tag_name": *"v\([^"]*\)".*/\1/'
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "https://api.github.com/repos/${SHINE_REPO}/releases/latest" \
      | grep '"tag_name"' \
      | sed 's/.*"tag_name": *"v\([^"]*\)".*/\1/'
  else
    fail "either curl or wget is required"
  fi
}

# Quote one literal argument for POSIX shells (also accepted by Bash and Zsh).
quote_arg() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

add_path_hint() {
  log "Warning: $SHINE_INSTALL_DIR is not in PATH"
  shell_name="${SHELL:-}"
  shell_name="${shell_name##*/}"
  case "$shell_name" in
    bash|zsh)
      if [ "$shell_name" = bash ]; then
        profile='"$HOME/.bashrc"'
      else
        # Resolve ZDOTDIR in the user's shell, where it may be set but not exported.
        profile='"${ZDOTDIR:-$HOME}/.zshrc"'
      fi
      escaped_dir="$(printf '%s' "$SHINE_INSTALL_DIR" | sed 's/[\\"$`]/\\&/g')"
      path_line="export PATH=\"$escaped_dir:\$PATH\""
      quoted_line="$(quote_arg "$path_line")"
      log "For $shell_name (from SHELL), run this command, then open a new shell:"
      log "  grep -Fqx -- $quoted_line $profile 2>/dev/null || printf '\\n%s\\n' $quoted_line >> $profile"
      ;;
    *)
      log "Automatic PATH configuration commands are only provided for Bash and Zsh (from SHELL)."
      log "Add the installation directory to PATH in your shell configuration, then open a new shell."
      ;;
  esac
}

main() {
  need_cmd tar
  target="$(detect_target)"
  if [ "$SHINE_VERSION" = "latest" ]; then
    log "Resolving latest version..."
    asset_version="$(resolve_version)"
    [ -n "$asset_version" ] || fail "could not resolve latest version from GitHub API"
    log "Latest version: v${asset_version}"
  else
    asset_version="$SHINE_VERSION"
  fi
  url="$(build_download_url "$asset_version" "$target")"

  tmpdir="$(mktemp -d 2>/dev/null || mktemp -d -t shine-install)"
  archive="$tmpdir/shine.tar.gz"

  trap 'rm -rf "$tmpdir"' EXIT INT TERM

  log "Downloading shine for ${target} from ${url}"
  download_file "$url" "$archive"

  mkdir -p "$SHINE_INSTALL_DIR"
  tar -xzf "$archive" -C "$tmpdir"

  [ -f "$tmpdir/shine" ] || fail "release archive did not contain a shine binary"

  install_path="$SHINE_INSTALL_DIR/shine"
  mv "$tmpdir/shine" "$install_path"
  chmod +x "$install_path"

  log "Installed shine to $install_path"
  case ":$PATH:" in
    *":$SHINE_INSTALL_DIR:"*) ;;
    *)
      add_path_hint
      ;;
  esac
}

main "$@"

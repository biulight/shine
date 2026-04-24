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

  if [ "$version" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download/%s' "$SHINE_REPO" "$asset"
  else
    printf 'https://github.com/%s/releases/download/v%s/%s' "$SHINE_REPO" "$version" "$asset"
  fi
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

main() {
  need_cmd tar
  target="$(detect_target)"
  if [ "$SHINE_VERSION" = "latest" ]; then
    asset_version="latest"
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
      log "Warning: $SHINE_INSTALL_DIR is not in PATH"
      log "Add this to your shell config:"
      log "  export PATH=\"$SHINE_INSTALL_DIR:\$PATH\""
      ;;
  esac
}

main "$@"

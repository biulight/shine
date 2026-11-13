#!/bin/sh

set -eu

SHINE_REPO="${SHINE_REPO:-biulight/shine}"
SHINE_INSTALL_DIR="${SHINE_INSTALL_DIR:-$HOME/.local/bin}"
SHINE_VERSION="${SHINE_VERSION:-latest}"

log() {
  printf '%s\n' "$*"
}

# Check the destination after redirection, including when fail writes to stderr.
# Plain log calls keep progress and copyable commands free of styling.
styled_log() {
  if [ -t 1 ] && [ -n "${TERM:-}" ] && [ "$TERM" != dumb ] && [ "${NO_COLOR+x}" != x ]; then
    printf '\033[%sm' "$1"
    shift
    printf '%s\033[0m\n' "$*"
  else
    shift
    log "$*"
  fi
}

fail() {
  styled_log 31 "error: $*" >&2
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
  if [ "$version" = "preview" ]; then
    printf 'https://github.com/%s/releases/download/preview/shine-preview-%s.tar.gz' "$SHINE_REPO" "$target"
    return
  fi
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
  styled_log 33 "Warning: $SHINE_INSTALL_DIR is not in PATH"
  shell_name="${SHELL:-}"
  shell_name="${shell_name##*/}"
  case "$shell_name" in
    bash|zsh)
      # Single-quote the literal path even in the current-terminal command:
      # interactive Bash expands history markers inside double quotes.
      quoted_dir="$(quote_arg "$SHINE_INSTALL_DIR")"
      path_line="case \":\$PATH:\" in *:$quoted_dir:*) ;; *) export PATH=$quoted_dir\"\${PATH:+:\$PATH}\" ;; esac"
      quoted_line="$(quote_arg "$path_line")"
      styled_log 1 "For $shell_name (from SHELL), run this command to configure future shells:"
      if [ "$shell_name" = bash ]; then
        # Select at command execution time. Never create a higher-priority profile
        # over an existing one, or source unrelated interactive configuration.
        log '  ( shine_profile="$HOME/.bash_profile";' \
          'for shine_file in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do if [ -e "$shine_file" ] || [ -L "$shine_file" ]; then shine_profile="$shine_file"; break; fi; done;' \
          'for shine_file in "$HOME/.bashrc" "$shine_profile"; do' \
          'if [ -e "$shine_file" ] || [ -L "$shine_file" ]; then' \
          'if [ ! -f "$shine_file" ] || [ ! -r "$shine_file" ] || [ ! -w "$shine_file" ]; then printf "Cannot configure PATH in %s; update it manually.\n" "$shine_file" >&2; exit 1; fi;' \
          'elif [ ! -w "$HOME" ]; then printf "Cannot create %s; update PATH manually.\n" "$shine_file" >&2; exit 1; fi; done;' \
          'for shine_file in "$HOME/.bashrc" "$shine_profile"; do' \
          "grep -Fqx -- $quoted_line \"\$shine_file\" 2>/dev/null || printf '\\n%s\\n' $quoted_line >> \"\$shine_file\" || exit 1; done )"
      else
        # Resolve ZDOTDIR in the user's shell, where it may be set but not exported.
        profile='"${ZDOTDIR:-$HOME}/.zshrc"'
        log "  grep -Fqx -- $quoted_line $profile 2>/dev/null || printf '\\n%s\\n' $quoted_line >> $profile"
      fi
      styled_log 1 "Then run this in your current terminal and verify with shine --version:"
      log "  $path_line"
      ;;
    *)
      log "Automatic PATH configuration commands are only provided for Bash and Zsh (from SHELL)."
      styled_log 1 "Add the installation directory to PATH in your shell configuration, then open a new shell."
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

  styled_log 32 "Installed shine to $install_path"
  case ":$PATH:" in
    *":$SHINE_INSTALL_DIR:"*) ;;
    *)
      add_path_hint
      ;;
  esac
}

main "$@"

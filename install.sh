#!/bin/sh

set -eu

SHINE_REPO="${SHINE_REPO:-biulight/shine}"
SHINE_INSTALL_DIR="${SHINE_INSTALL_DIR:-$HOME/.local/bin}"
SHINE_VERSION="${SHINE_VERSION:-latest}"

log() {
  printf '%s\n' "$*"
}

# Check the destination after redirection, including when fail writes to stderr.
# Redirected output and terminals opting out of color receive plain text.
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
    if [ -t 2 ] && [ "${TERM:-}" != dumb ]; then
      curl -fSL --progress-bar "$url" -o "$dest"
    else
      curl -fsSL "$url" -o "$dest"
    fi
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    # Let GNU/BusyBox wget show native progress when supported by the build.
    # Avoid GNU-only --show-progress / --progress options.
    if [ -t 2 ] && [ "${TERM:-}" != dumb ]; then
      wget -O "$dest" "$url"
    else
      wget -qO "$dest" "$url"
    fi
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

# Keep ordinary paths readable without exposing shell metacharacters.
compact_arg() {
  case "$1" in
    ''|*[!a-zA-Z0-9_./-]*) quote_arg "$1" ;;
    *) printf '%s' "$1" ;;
  esac
}

# Recognize common interactive startup links without evaluating user code.
# This is deliberately not a parser for arbitrary shell control flow.
bash_profile_loads_rc() {
  [ -f "$1" ] && [ -r "$1" ] || return 1
  rc_reference='(\$(HOME|\{HOME\})/\.bashrc|"\$(HOME|\{HOME\})/\.bashrc"|~/\.bashrc)'
  rc_source="(\\.|source)[[:space:]]+$rc_reference"
  rc_check="\\[[[:space:]]+-[fr][[:space:]]+${rc_reference}[[:space:]]+\\]"
  rc_negative_check="\\[[[:space:]]+![[:space:]]+-[fr][[:space:]]+${rc_reference}[[:space:]]+\\]"
  rc_guard="(${rc_check}[[:space:]]*&&|${rc_negative_check}[[:space:]]*\\|\\|)"
  # A hash attached to a word is part of that word, not a shell comment.
  rc_end='([[:space:]]*;[[:space:]]*(#.*)?|[[:space:]]+#.*|[[:space:]]*)$'
  grep -Eq \
    -e "^[[:space:]]*(${rc_guard}[[:space:]]*)?${rc_source}${rc_end}" \
    -e "^[[:space:]]*if[[:space:]]+${rc_check}[[:space:]]*;[[:space:]]*then[[:space:]]+${rc_source}[[:space:]]*;[[:space:]]*fi${rc_end}" \
    -- "$1"
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
      legacy_path_line="case \":\$PATH:\" in *:$quoted_dir:*) ;; *) export PATH=$quoted_dir\"\${PATH:+:\$PATH}\" ;; esac"
      quoted_dir="$(compact_arg "$SHINE_INSTALL_DIR")"
      guarded_path_line="case \":\$PATH:\" in *:$quoted_dir:*) ;; *) export PATH=$quoted_dir\"\${PATH:+:\$PATH}\" ;; esac"
      path_line="export PATH=$quoted_dir\"\${PATH:+:\$PATH}\""
      if [ "$shell_name" = bash ]; then
        # Inspect only: configuration is still written by the user-run hint.
        profile="$HOME/.bash_profile"
        for candidate in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
          if [ -e "$candidate" ] || [ -L "$candidate" ]; then
            profile="$candidate"
            break
          fi
        done
        set -- "$HOME/.bashrc" "$profile"
        if bash_profile_loads_rc "$profile"; then
          set -- "$HOME/.bashrc"
        fi
      else
        # Only exported ZDOTDIR is visible; print the observed literal destination.
        profile="${ZDOTDIR:-$HOME}/.zshrc"
        set -- "$profile"
      fi
      profiles_ready=true
      for target_profile do
        if [ -e "$target_profile" ] || [ -L "$target_profile" ]; then
          if [ ! -f "$target_profile" ] || [ ! -r "$target_profile" ] || [ ! -w "$target_profile" ]; then
            styled_log 33 "Cannot configure PATH in $target_profile; update it manually."
            profiles_ready=false
          fi
        elif [ ! -w "${target_profile%/*}" ] || [ ! -x "${target_profile%/*}" ]; then
          styled_log 33 "Cannot create $target_profile; update PATH manually."
          profiles_ready=false
        fi
      done
      if [ "$profiles_ready" = true ]; then
        hint_command=""
        first_target=""
        quoted_line="$(quote_arg "$path_line")"
        for target_profile do
          # Recognize older guarded statements without evaluating user configuration.
          if grep -Fqx -e "$path_line" -e "$legacy_path_line" -e "$guarded_path_line" -- "$target_profile" 2>/dev/null; then
            continue
          fi
          # Shared symlink or hard-link targets must receive only one append.
          if [ -n "$first_target" ] && [ "$target_profile" -ef "$first_target" ]; then
            continue
          fi
          quoted_profile="$(compact_arg "$target_profile")"
          if [ "$shell_name" = bash ] && [ "$target_profile" = "$profile" ] && [ ! -e "$profile" ] && [ ! -L "$profile" ]; then
            # Only a newly created login file receives a .bashrc link.
            login_line='case $- in *i*) [ ! -r "$HOME/.bashrc" ] || . "$HOME/.bashrc" ;; esac'
            hint_command="${hint_command:+$hint_command && }printf '\\n%s\\n' $quoted_line $(quote_arg "$login_line") >> $quoted_profile"
          elif [ -n "$first_target" ]; then
            hint_command="printf '\\n%s\\n' $quoted_line | tee -a -- $(compact_arg "$first_target") $quoted_profile >/dev/null"
          else
            hint_command="printf '\\n%s\\n' $quoted_line >> $quoted_profile"
          fi
          first_target="$target_profile"
        done
        if [ -n "$hint_command" ]; then
          log ""
          styled_log 1 "For $shell_name (from SHELL), run this command to configure future shells:"
          log ""
          styled_log 36 "  $hint_command"
        fi
      fi
      log ""
      styled_log 1 "Then run this in your current terminal and verify with shine --version:"
      log ""
      styled_log 36 "  $path_line"
      ;;
    *)
      log "Automatic PATH configuration commands are only provided for Bash and Zsh (from SHELL)."
      styled_log 1 "Add the installation directory to PATH in your shell configuration, then open a new shell."
      ;;
  esac
}

main() {
  # Generated PATH statements and copyable commands must fit on one line.
  newline='
'
  carriage_return="$(printf '\r')"
  case "$HOME" in
    *"$newline"*|*"$carriage_return"*) fail "HOME paths containing newline characters are not supported" ;;
  esac
  case "$SHINE_INSTALL_DIR" in
    *"$newline"*|*"$carriage_return"*) fail "SHINE_INSTALL_DIR paths containing newline characters are not supported" ;;
  esac
  case "${SHELL:-}" in
    zsh|*/zsh)
      case "${ZDOTDIR:-}" in
        *"$newline"*|*"$carriage_return"*) fail "ZDOTDIR paths containing newline characters are not supported" ;;
      esac
      ;;
  esac
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

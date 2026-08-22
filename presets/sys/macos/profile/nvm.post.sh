nvm() {
  unfunction nvm 2>/dev/null
  if [[ -n "${HOMEBREW_PREFIX:-}" && -s "$HOMEBREW_PREFIX/opt/nvm/nvm.sh" ]]; then
    source "$HOMEBREW_PREFIX/opt/nvm/nvm.sh"
  elif [[ -s "$NVM_DIR/nvm.sh" ]]; then
    source "$NVM_DIR/nvm.sh"
  fi
  nvm "$@"
}

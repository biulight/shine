#!/bin/bash
set -euo pipefail
test ! -e "$SHINE_TARGET_HOME/.config/nvim"
mkdir -p "$SHINE_TARGET_HOME/.config"
git clone --depth 1 https://github.com/AstroNvim/template "$SHINE_TARGET_HOME/.config/nvim"
rm -rf "$SHINE_TARGET_HOME/.config/nvim/.git"

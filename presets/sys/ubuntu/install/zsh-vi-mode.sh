#!/bin/bash
set -euo pipefail
mkdir -p "$SHINE_TARGET_HOME/.local/share"
git clone --depth 1 https://github.com/jeffreytse/zsh-vi-mode.git "$SHINE_TARGET_HOME/.local/share/zsh-vi-mode"

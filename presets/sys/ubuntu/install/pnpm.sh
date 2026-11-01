#!/bin/bash
set -euo pipefail
export PNPM_HOME="${SHINE_TARGET_HOME:-$HOME}/.local/share/pnpm"
# Shine owns shell integration; upstream must only install pnpm.
export PNPM_NO_MODIFY_PATH=1
curl -fsSL https://get.pnpm.io/install.sh | sh -

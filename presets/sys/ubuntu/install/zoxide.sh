#!/bin/bash
set -euo pipefail
mkdir -p "$SHINE_TARGET_HOME/.local/bin"
curl -sSfL https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/install.sh | sh

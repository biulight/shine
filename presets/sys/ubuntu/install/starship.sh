#!/bin/bash
set -euo pipefail
curl -sS https://starship.rs/install.sh | sh -s -- -y -b "$SHINE_TARGET_HOME/.local/bin"

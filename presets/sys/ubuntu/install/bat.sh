#!/bin/bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y bat
mkdir -p "$SHINE_TARGET_HOME/.local/bin"
ln -sf /usr/bin/batcat "$SHINE_TARGET_HOME/.local/bin/bat"

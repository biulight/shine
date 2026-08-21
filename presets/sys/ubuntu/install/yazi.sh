#!/bin/bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y file ffmpeg 7zip jq poppler-utils fd-find ripgrep fzf imagemagick xclip
curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
"$SHINE_TARGET_HOME/.cargo/bin/cargo" install --locked yazi-fm yazi-cli

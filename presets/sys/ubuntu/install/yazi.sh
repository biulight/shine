#!/bin/bash
set -euo pipefail
# Download completely before replacing the trusted keyring; keep it readable by APT.
keyring="$(mktemp)"
trap 'rm -f "$keyring"' EXIT
curl -fsSL https://yazi-rs.github.io/builds/yazi-keyring.gpg -o "$keyring"
sudo install -m 0644 "$keyring" /usr/share/keyrings/yazi-keyring.gpg
echo 'deb [signed-by=/usr/share/keyrings/yazi-keyring.gpg] https://yazi-rs.github.io/builds/ stable main' | sudo tee /etc/apt/sources.list.d/yazi.list >/dev/null
sudo apt-get update --error-on=any
sudo apt-get install -y yazi file ffmpeg 7zip jq poppler-utils fd-find ripgrep fzf imagemagick xclip

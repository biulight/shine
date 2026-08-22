#!/bin/bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y gpg
mkdir -p /etc/apt/keyrings
wget -qO- https://raw.githubusercontent.com/eza-community/eza/main/deb.asc | sudo gpg --dearmor -o /etc/apt/keyrings/gierens.gpg
echo 'deb [signed-by=/etc/apt/keyrings/gierens.gpg] http://deb.gierens.de stable main' | sudo tee /etc/apt/sources.list.d/gierens.list >/dev/null
sudo apt-get update
sudo apt-get install -y eza

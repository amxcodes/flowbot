#!/bin/bash
set -e

echo -e "\033[0;36m🚀 Starting Flowbot Installer...\033[0m"

# 1. Check for Rust/Cargo
if ! command -v cargo &> /dev/null; then
    echo -e "\033[0;33m⚠️  Rust is not installed.\033[0m"
    echo "This script can install Rust for you (via rustup.rs)."
    read -p "Do you want to install Rust now? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo -e "\033[0;36m📥 Installing Rust...\033[0m"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # Source env to use cargo immediately
        source "$HOME/.cargo/env"
    else
        echo "Aborting. Rust is required."
        exit 1
    fi
fi

# 2. Build & Install via Cargo
echo -e "\033[0;36m📦 Building and Installing Flowbot...\033[0m"
# --force ensures we overwrite any old version
cargo install --path . --force

# 3. Setup Config Directory
mkdir -p "$HOME/.nanobot"

echo -e "\n\033[0;32m✨ Installation Complete!\033[0m"
echo -e "You can now run: \033[1mflowbot doctor\033[0m"

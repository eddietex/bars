#!/bin/sh
# Native (non-Flatpak) install into ~/.local for development.
# Build first: cargo build --release -p bars
set -eu
cd "$(dirname "$(readlink -f "$0")")/.."
install -Dm755 target/release/bars "$HOME/.local/bin/bars"
install -Dm644 packaging/linux/xyz.hink.Bars.desktop "$HOME/.local/share/applications/xyz.hink.Bars.desktop"
sed -i "s|^Exec=bars|Exec=$HOME/.local/bin/bars|" "$HOME/.local/share/applications/xyz.hink.Bars.desktop"
for size in 32 48 64 128 256 512; do
    install -Dm644 "packaging/linux/icons/${size}x${size}/xyz.hink.Bars.png" \
        "$HOME/.local/share/icons/hicolor/${size}x${size}/apps/xyz.hink.Bars.png"
done
echo "Installed to ~/.local/bin/bars"

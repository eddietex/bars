#!/usr/bin/env bash
# Builds the DMG the way releases do (from the debug binary, to save time),
# opens it in Finder and takes a screenshot of its window.
set -euo pipefail
mkdir -p smoke target/release
cp target/debug/bars target/release/bars
dmg=$(bash scripts/package-macos.sh | tail -1)
echo "built $dmg"
mount=$(hdiutil attach "$dmg" | awk -F'\t' '/\/Volumes\//{print $NF}' | tail -1)
echo "mounted at $mount"
ls -la "$mount" "$mount/.background"
[ -f "$mount/.DS_Store" ] || { echo "no Finder layout (.DS_Store) in the DMG"; exit 1; }
open "$mount"
sleep 4
screencapture -x smoke/5-dmg.png || true
osascript -e 'tell application "Finder" to close every window' >/dev/null 2>&1 || true
hdiutil detach "$mount" -quiet || true

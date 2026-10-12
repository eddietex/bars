#!/usr/bin/env bash
# Builds dist/Bars_<version>_<arch>.dmg from target/release/bars.
#
# cargo-packager makes Bars.app; dmgbuild makes the disk image. (cargo-
# packager's own DMG step uses create-dmg, which skips the Finder styling
# (background, icon positions) whenever CI=true, as on GitHub Actions.
# dmgbuild writes that layout itself, without driving Finder.)
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(uname -m)
[ "$arch" = arm64 ] && arch=aarch64

# Retina background: 1x and 2x in one TIFF.
tiffutil -cathidpicheck packaging/macos/dmg-background.png \
  packaging/macos/dmg-background@2x.png -out packaging/macos/dmg-background.tiff

rm -rf dist/*.app
cargo packager --release -p bars --formats app
app=$(ls -d dist/*.app | head -1)

# The Mac icon: macOS 26 shows any icon not shaped like its own rounded
# square on a grey plate, so the Mac gets packaging/macos/icon.png (made by
# scripts/macos-icon.swift) instead of the Linux icons cargo-packager uses.
iconset=$(mktemp -d)/Bars.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size packaging/macos/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) packaging/macos/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
icon_file=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$app/Contents/Info.plist")
iconutil -c icns "$iconset" -o "$app/Contents/Resources/$icon_file"

# Sign the whole app, not just the executable (the linker's ad hoc
# signature leaves Info.plist and resources unsealed, which macOS reports
# as "damaged" for a downloaded copy). CI signs with the self-signed
# certificate (scripts/macos-signing-keychain.sh): one identity across
# releases keeps Accessibility and Microphone access through updates.
# Elsewhere: ad hoc.
identity="${BARS_SIGNING_IDENTITY:--}"
codesign --force --sign "$identity" --timestamp=none "$app"
codesign --verify --strict --verbose=2 "$app"
codesign --display --requirements - "$app"

# The in-app updater's download: the app as signed above, so an update
# keeps the code signature (and with it Accessibility and Microphone
# access), plus a signature with the updater key (CI secret). Made here
# rather than by cargo-packager, whose archive is taken before codesign.
if [ -n "${BARS_UPDATER_KEY:-}" ]; then
  archive="dist/Bars_${version}_${arch}.app.tar.gz"
  COPYFILE_DISABLE=1 tar -czf "$archive" -C "$(dirname "$app")" "$(basename "$app")"
  cargo packager signer sign -k "$BARS_UPDATER_KEY" --password "${BARS_UPDATER_KEY_PASSWORD:-}" "$archive"
fi

if [ ! -x .dmgbuild/bin/dmgbuild ]; then
  python3 -m venv .dmgbuild
  .dmgbuild/bin/pip install --quiet 'dmgbuild>=1.6,<2'
fi
dmg="dist/Bars_${version}_${arch}.dmg"
rm -f "$dmg"
.dmgbuild/bin/dmgbuild -s packaging/macos/dmg_settings.py \
  -D app="$app" -D background=packaging/macos/dmg-background.tiff \
  "Bars" "$dmg"
echo "$dmg"

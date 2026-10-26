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
# as "damaged" for a downloaded copy), with the hardened runtime that
# notarization requires. CI signs with the Developer ID certificate
# (scripts/macos-signing-keychain.sh). Elsewhere: ad hoc, which can't carry
# a timestamp.
identity="${BARS_SIGNING_IDENTITY:--}"
timestamp=--timestamp
[ "$identity" = - ] && timestamp=--timestamp=none
codesign --force --sign "$identity" "$timestamp" --options runtime \
  --entitlements packaging/macos/Bars.entitlements "$app"
codesign --verify --strict --verbose=2 "$app"
codesign --display --requirements - "$app"

# Notarized (CI, with the App Store Connect API key in $APPLE_API_KEY): Apple
# checks the upload, and its ticket is stapled on, so macOS opens Bars
# without a warning, offline too.
notarize() {
  local out id status
  out=$(xcrun notarytool submit "$1" --key "$api_key" --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" --wait --output-format json) || true
  id=$(printf '%s' "$out" | python3 -c 'import json, sys; print(json.load(sys.stdin)["id"])')
  status=$(printf '%s' "$out" | python3 -c 'import json, sys; print(json.load(sys.stdin)["status"])')
  echo "notarization of $(basename "$1"): $status ($id)"
  if [ "$status" != Accepted ]; then
    xcrun notarytool log "$id" --key "$api_key" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID"
    return 1
  fi
}
if [ -n "${APPLE_API_KEY:-}" ]; then
  : "${APPLE_API_KEY_ID:?}" "${APPLE_API_ISSUER_ID:?}"
  api_key=$(mktemp -d)/AuthKey.p8
  printf '%s\n' "$APPLE_API_KEY" > "$api_key"
  zip=$(mktemp -d)/Bars.zip
  ditto -c -k --keepParent "$app" "$zip"
  notarize "$zip"
  xcrun stapler staple "$app"
  spctl --assess --type execute --verbose=2 "$app"
fi

# The in-app updater's download: the app as signed (and stapled) above, so
# an update keeps the code signature (and with it Accessibility and
# Microphone access), plus a signature with the updater key (CI secret).
# Made here rather than by cargo-packager, whose archive is taken before
# codesign.
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
if [ "$identity" != - ]; then
  codesign --force --sign "$identity" --timestamp "$dmg"
fi
if [ -n "${APPLE_API_KEY:-}" ]; then
  notarize "$dmg"
  xcrun stapler staple "$dmg"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
fi
echo "$dmg"

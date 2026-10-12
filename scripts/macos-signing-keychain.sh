#!/usr/bin/env bash
# CI only: puts the self-signed code-signing certificate (a base64 .p12 in
# $P12, its password in $P12_PASSWORD) into a temporary keychain, trusts it
# for code signing on this runner, and exports BARS_SIGNING_IDENTITY for
# package-macos.sh. Without $P12 it does nothing, and the app is signed ad
# hoc instead.
#
# The certificate is self-signed, not an Apple Developer ID: a downloaded
# Bars still needs "Open Anyway" once, but every release carries the same
# identity, so macOS keeps Accessibility and Microphone access across
# updates.
set -euo pipefail

if [ -z "${P12:-}" ]; then
  echo "No signing certificate; Bars.app will be signed ad hoc."
  exit 0
fi

dir="${RUNNER_TEMP:-$(mktemp -d)}"
keychain="$dir/bars-signing.keychain-db"
keychain_password=$(openssl rand -hex 16)

printf '%s' "$P12" | base64 --decode > "$dir/bars.p12"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$dir/bars.p12" -k "$keychain" -P "$P12_PASSWORD" -T /usr/bin/codesign
# Lets codesign use the key without a password prompt.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')

# A self-signed certificate isn't trusted by default.
openssl pkcs12 -in "$dir/bars.p12" -nokeys -passin env:P12_PASSWORD -out "$dir/bars.pem"
sudo security add-trusted-cert -d -r trustRoot -p codeSign -k /Library/Keychains/System.keychain "$dir/bars.pem"
rm -f "$dir/bars.p12"

identity=$(openssl x509 -in "$dir/bars.pem" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g')
security find-identity -p codesigning "$keychain"
echo "BARS_SIGNING_IDENTITY=$identity" >> "$GITHUB_ENV"
echo "Signing as $identity"

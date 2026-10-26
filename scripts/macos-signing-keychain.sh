#!/usr/bin/env bash
# CI only: puts the Developer ID Application certificate (a base64 .p12 in
# $P12 with Apple's intermediate, its password in $P12_PASSWORD) into a
# temporary keychain and exports BARS_SIGNING_IDENTITY for
# package-macos.sh. Without $P12 it does nothing, and the app is signed ad
# hoc instead.
#
# macOS keys Bars' Accessibility and Microphone access to the Team ID in the
# certificate, so they last across updates and certificate renewals.
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

# The certificate that goes with the key, not Apple's intermediate.
openssl pkcs12 -in "$dir/bars.p12" -clcerts -nokeys -passin env:P12_PASSWORD -out "$dir/bars.pem"
rm -f "$dir/bars.p12"

identity=$(openssl x509 -in "$dir/bars.pem" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g')
openssl x509 -in "$dir/bars.pem" -noout -subject -enddate
security find-identity -v -p codesigning "$keychain"
echo "BARS_SIGNING_IDENTITY=$identity" >> "$GITHUB_ENV"
echo "Signing as $identity"

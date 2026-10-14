#!/bin/sh
# Builds and installs the Bars Flatpak for the current user.
# Requires org.flatpak.Builder and the rust-stable and llvm22 SDK extensions:
#   flatpak install --user flathub org.flatpak.Builder \
#       org.freedesktop.Sdk//26.08 org.freedesktop.Sdk.Extension.rust-stable//26.08 \
#       org.freedesktop.Sdk.Extension.llvm22//26.08
set -eu
cd "$(dirname "$(readlink -f "$0")")/.."
if command -v flatpak-builder >/dev/null 2>&1; then
    FB=flatpak-builder
    # Inside toolbox/distrobox: use the host's system runtimes too.
    if [ -d /run/host/var/lib/flatpak ]; then
        [ -z "${FLATPAK_SYSTEM_DIR:-}" ] && export FLATPAK_SYSTEM_DIR=/run/host/var/lib/flatpak
        # No FUSE inside containers.
        FB="$FB --disable-rofiles-fuse"
    fi
else
    FB="flatpak run --command=flatpak-builder org.flatpak.Builder"
fi
$FB --user --install --force-clean --ccache \
    --state-dir=.flatpak-builder build-dir packaging/flatpak/xyz.hink.Bars.yml
# Optional: produce a single-file bundle to share.
if [ "${BUNDLE:-0}" = 1 ]; then
    $FB --user --repo=repo --force-clean --state-dir=.flatpak-builder build-dir packaging/flatpak/xyz.hink.Bars.yml
    flatpak build-bundle repo bars.flatpak xyz.hink.Bars
fi

#!/usr/bin/env bash
# Build md-wingman from source, package it as md-wingman.app and install it.
#
#   scripts/install.sh
#
# Environment:
#   WINGMAN_INSTALL_DIR        where to put md-wingman.app; defaults to
#                              /Applications, or ~/Applications when
#                              /Applications isn't writable
#   WINGMAN_CODESIGN_IDENTITY  the identity to sign with; defaults to your
#                              "Apple Development" certificate when there is
#                              one, else "-" (ad-hoc)
#
# The app you build here never leaves your Mac, so macOS doesn't quarantine it
# and Gatekeeper lets it open without a Developer ID signature. Signing with
# your own Apple Development certificate (free with an Apple ID in Xcode) also
# keeps "Always Allow" for the TypeSafe key in the Keychain across rebuilds;
# an ad-hoc signature changes with every build, so macOS asks again.
#
# The building, packaging and signing are in scripts/package.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname)" != Darwin ]]; then
    echo "install.sh: md-wingman.app can only be built on macOS" >&2
    exit 1
fi

app_name=md-wingman.app
dest="${WINGMAN_INSTALL_DIR:-}"
if [[ -z "$dest" ]]; then
    if [[ -w /Applications ]]; then dest=/Applications; else dest="$HOME/Applications"; fi
fi

app="$(scripts/package.sh | tail -n 1)"

echo "==> Installing to $dest"
mkdir -p "$dest"
rm -rf "${dest:?}/$app_name"
ditto "$app" "$dest/$app_name"

echo "Installed $dest/$app_name"
echo "Open it with: open \"$dest/$app_name\""

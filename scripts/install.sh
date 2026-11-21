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
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname)" != Darwin ]]; then
    echo "install.sh: md-wingman.app can only be built on macOS" >&2
    exit 1
fi

app_name=md-wingman.app
bundle_id=io.github.deanberris.md-wingman
version="$(awk -F'"' '/^version *=/{print $2; exit}' Cargo.toml)"

dest="${WINGMAN_INSTALL_DIR:-}"
if [[ -z "$dest" ]]; then
    if [[ -w /Applications ]]; then dest=/Applications; else dest="$HOME/Applications"; fi
fi

identity="${WINGMAN_CODESIGN_IDENTITY:-}"
if [[ -z "$identity" ]]; then
    # The first valid Apple Development identity, by its SHA-1 hash.
    identity="$(security find-identity -v -p codesigning 2>/dev/null |
        awk '/"Apple Development: /{print $2; exit}')"
fi
if [[ -z "$identity" ]]; then
    identity=-
    echo "install.sh: no Apple Development certificate, signing ad-hoc" >&2
fi

echo "==> Building the assistant service"
(cd service && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)

echo "==> Building md-wingman (release)"
cargo build --release -p md-wingman --locked

echo "==> Packaging $app_name"
stage="target/release/bundle"
app="$stage/$app_name"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/md-wingman "$app/Contents/MacOS/md-wingman"
cp service/dist/wingman-service "$app/Contents/Resources/wingman-service"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>md-wingman</string>
    <key>CFBundleExecutable</key><string>md-wingman</string>
    <key>CFBundleIdentifier</key><string>$bundle_id</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>md-wingman</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "==> Signing with ${identity/#-/ad-hoc}"
# Inside out: the nested service first, then the app bundle itself.
codesign --force --sign "$identity" --identifier "$bundle_id.service" \
    "$app/Contents/Resources/wingman-service"
codesign --force --sign "$identity" --identifier "$bundle_id" "$app"
codesign --verify --strict --deep "$app"

echo "==> Installing to $dest"
mkdir -p "$dest"
rm -rf "${dest:?}/$app_name"
ditto "$app" "$dest/$app_name"

echo "Installed $dest/$app_name"
echo "Open it with: open \"$dest/$app_name\""

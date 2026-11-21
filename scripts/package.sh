#!/usr/bin/env bash
# Build md-wingman from source and package it as a signed md-wingman.app in
# target/release/bundle/. scripts/install.sh and the release workflow both
# use it.
#
#   scripts/package.sh
#
# Environment:
#   WINGMAN_CODESIGN_IDENTITY  the identity to sign with; defaults to your
#                              "Apple Development" certificate when there is
#                              one, else "-" (ad-hoc)
#   WINGMAN_HARDENED_RUNTIME   set to 1 to sign with the hardened runtime and
#                              a secure timestamp, as notarization requires.
#                              Only for Developer ID builds.
#
# Prints the path of the packaged app as its last line.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname)" != Darwin ]]; then
    echo "package.sh: md-wingman.app can only be built on macOS" >&2
    exit 1
fi

app_name=md-wingman.app
bundle_id=io.github.deanberris.md-wingman
version="$(awk -F'"' '/^version *=/{print $2; exit}' Cargo.toml)"

identity="${WINGMAN_CODESIGN_IDENTITY:-}"
if [[ -z "$identity" ]]; then
    # The first valid Apple Development identity, by its SHA-1 hash.
    identity="$(security find-identity -v -p codesigning 2>/dev/null |
        awk '/"Apple Development: /{print $2; exit}')"
fi
if [[ -z "$identity" ]]; then
    identity=-
    echo "package.sh: no Apple Development certificate, signing ad-hoc" >&2
fi

sign_opts=(--force --sign "$identity")
service_opts=()
if [[ "${WINGMAN_HARDENED_RUNTIME:-}" == 1 ]]; then
    sign_opts+=(--options runtime --timestamp)
    # The Bun-compiled service runs JavaScriptCore's JIT, which the hardened
    # runtime blocks unless these entitlements allow it.
    service_opts=(--entitlements scripts/service.entitlements)
fi

echo "==> Building the assistant service" >&2
(cd service && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)

echo "==> Building md-wingman (release)" >&2
cargo build --release -p md-wingman --locked >&2

echo "==> Packaging $app_name" >&2
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

echo "==> Signing with ${identity/#-/ad-hoc}" >&2
# Inside out: the nested service first, then the app bundle itself.
codesign "${sign_opts[@]}" ${service_opts[@]+"${service_opts[@]}"} \
    --identifier "$bundle_id.service" "$app/Contents/Resources/wingman-service"
codesign "${sign_opts[@]}" --identifier "$bundle_id" "$app"
codesign --verify --strict --deep "$app"

echo "$app"

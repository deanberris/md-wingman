#!/usr/bin/env bash
# Make the drag-to-Applications disk image for a packaged md-wingman.app.
# The release workflow uses it; it works locally too.
#
#   scripts/make-dmg.sh path/to/md-wingman.app path/to/out.dmg
#
# Environment:
#   WINGMAN_CODESIGN_IDENTITY  sign the disk image with this identity, unless
#                              it is unset or "-" (ad-hoc images aren't signed)
#
# Needs python3. dmgbuild is installed into a throwaway virtualenv, pinned.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname)" != Darwin ]]; then
    echo "make-dmg.sh: disk images can only be made on macOS" >&2
    exit 1
fi
if [[ $# -ne 2 || ! -d "$1" ]]; then
    echo "usage: scripts/make-dmg.sh path/to/md-wingman.app path/to/out.dmg" >&2
    exit 2
fi
app="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
out="$2"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "==> Installing dmgbuild" >&2
python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet --disable-pip-version-check dmgbuild==1.6.7

# One TIFF with both resolutions, so the window is sharp on Retina displays.
tiffutil -cathidpicheck scripts/dmg/background.png scripts/dmg/background@2x.png \
    -out "$work/background.tiff" 2>/dev/null

echo "==> Making $out" >&2
rm -f "$out"
"$work/venv/bin/dmgbuild" -s scripts/dmg/settings.py \
    -D app="$app" -D background="$work/background.tiff" \
    md-wingman "$out"

identity="${WINGMAN_CODESIGN_IDENTITY:--}"
if [[ "$identity" != - ]]; then
    codesign --force --sign "$identity" --timestamp "$out"
fi

# The image must mount, hold an app that still verifies, and link to
# Applications.
mount="$work/mnt"
mkdir -p "$mount"
hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$out" >/dev/null
trap 'hdiutil detach -quiet "$mount" || true; rm -rf "$work"' EXIT
codesign --verify --strict --deep "$mount/$(basename "$app")"
test "$(readlink "$mount/Applications")" = /Applications
test -f "$mount/.DS_Store"
hdiutil detach -quiet "$mount"
trap 'rm -rf "$work"' EXIT

echo "$out"

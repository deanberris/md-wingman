#!/usr/bin/env bash
# Build the assistant service and run md-wingman from source.
#
#   scripts/dev.sh [FILE]
#
# Environment:
#   WINGMAN_AUTH=api_key   use ANTHROPIC_API_KEY instead of your Claude subscription
#   WINGMAN_FAKE=1         canned assistant replies, no Claude calls
#   WINGMAN_CODESIGN_IDENTITY  the certificate to sign the build with; defaults
#                          to "md-wingman dev" when your Keychain has one
#
# Signing every build with the same certificate keeps macOS's "Always Allow"
# for the TypeSafe key in the Keychain. Unsigned and ad-hoc signed builds get
# a new identity each time they're rebuilt, so macOS asks again.
set -euo pipefail
cd "$(dirname "$0")/.."

(cd service && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)
cargo build --release -p md-wingman

bin=target/release/md-wingman
identity="${WINGMAN_CODESIGN_IDENTITY:-}"
if [[ -z "$identity" && "$(uname)" == Darwin ]] &&
    security find-identity -v -p codesigning 2>/dev/null | grep -q '"md-wingman dev"'; then
    identity="md-wingman dev"
fi
if [[ -n "$identity" ]]; then
    codesign --force --sign "$identity" --identifier io.github.deanberris.md-wingman "$bin"
fi
exec "$bin" "$@"

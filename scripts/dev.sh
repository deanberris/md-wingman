#!/usr/bin/env bash
# Build the assistant service and run md-wingman from source.
#
#   scripts/dev.sh [FILE]
#
# Environment:
#   WINGMAN_AUTH=api_key   use ANTHROPIC_API_KEY instead of your Claude subscription
#   WINGMAN_FAKE=1         canned assistant replies, no Claude calls
#   WINGMAN_CODESIGN_IDENTITY  the identity to sign the build with; defaults to
#                          your "Apple Development" certificate when there is one
#
# Signing every build with the same certificate keeps macOS's "Always Allow"
# for the TypeSafe key in the Keychain. Unsigned and ad-hoc signed builds get
# a new identity each time they're rebuilt, so macOS asks again. Signing in to
# Xcode with your Apple ID (Settings > Accounts) creates an Apple Development
# certificate; a free account is enough.
set -euo pipefail
cd "$(dirname "$0")/.."

(cd service && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)
cargo build --release -p md-wingman

bin=target/release/md-wingman
identity="${WINGMAN_CODESIGN_IDENTITY:-}"
if [[ -z "$identity" && "$(uname)" == Darwin ]]; then
    # The first valid Apple Development identity, by its SHA-1 hash.
    identity="$(security find-identity -v -p codesigning 2>/dev/null |
        awk '/"Apple Development: /{print $2; exit}')"
fi
if [[ -n "$identity" ]]; then
    codesign --force --sign "$identity" --identifier io.github.deanberris.md-wingman "$bin"
elif [[ "$(uname)" == Darwin ]]; then
    echo "dev.sh: no Apple Development certificate, so macOS may ask for Keychain access after each rebuild" >&2
fi
exec "$bin" "$@"

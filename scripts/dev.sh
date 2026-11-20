#!/usr/bin/env bash
# Build the assistant service and run md-wingman from source.
#
#   scripts/dev.sh [FILE]
#
# Environment:
#   WINGMAN_AUTH=api_key   use ANTHROPIC_API_KEY instead of your Claude subscription
#   WINGMAN_FAKE=1         canned assistant replies, no Claude calls
set -euo pipefail
cd "$(dirname "$0")/.."

(cd service && bun install --frozen-lockfile >/dev/null && bun run build >/dev/null)
exec cargo run --release -p md-wingman -- "$@"

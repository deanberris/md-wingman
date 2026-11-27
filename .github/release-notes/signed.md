## Install

Needs an Apple Silicon Mac, and [Claude Code](https://claude.com/claude-code) installed and signed in (`claude auth
login`). md-wingman uses your own Claude subscription through it.

1. Download `md-wingman-{{VERSION}}-macos-arm64.dmg` (or the `.zip`) below.
2. Open the disk image and drag md-wingman to Applications.
3. Open md-wingman.

This build is signed with a Developer ID and notarized by Apple. md-wingman downloads later versions in the background
and installs them when you quit; a note in the status bar says when one is ready.

## Verify the download

Each file is attested by GitHub: the attestation proves it was built by this repository's release workflow from tag
`{{TAG}}`. With the [GitHub CLI](https://cli.github.com):

```sh
gh attestation verify md-wingman-{{VERSION}}-macos-arm64.dmg --repo deanberris/md-wingman
```

Or compare checksums with `SHA256SUMS.txt`:

```sh
shasum -a 256 -c SHA256SUMS.txt --ignore-missing
```

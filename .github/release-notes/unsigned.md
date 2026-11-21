> **Unsigned, for early adopters.** This build is signed ad-hoc, not with an Apple Developer ID, and isn't notarized by Apple. macOS will stop it the first time you open it. If you'd rather not approve an unsigned app, [build it from source](https://github.com/deanberris/md-wingman#run-it-on-a-mac) instead.

## Install

Needs an Apple Silicon Mac, and [Claude Code](https://claude.com/claude-code) installed and signed in (`claude auth login`). md-wingman uses your own Claude subscription through it.

1. Download `md-wingman-{{VERSION}}-macos-arm64.dmg` (or the `.zip`) below.
2. Open the disk image and drag md-wingman to Applications.
3. Open md-wingman. macOS says it can't check the app for malicious software. Choose **Done**, not Move to Trash.
4. Open **System Settings ▸ Privacy & Security**, scroll down to the message about md-wingman, and choose **Open Anyway**. Confirm with your password or Touch ID. You only do this once per version.

If you prefer Terminal, removing the quarantine flag does the same as step 4:

```sh
xattr -d com.apple.quarantine /Applications/md-wingman.app
```

Do this only for a download you've verified (below), because it skips the check macOS would otherwise do.

## Verify the download

Each file is attested by GitHub: the attestation proves it was built by this repository's release workflow from tag `{{TAG}}`, not uploaded by hand. With the [GitHub CLI](https://cli.github.com):

```sh
gh attestation verify md-wingman-{{VERSION}}-macos-arm64.dmg --repo deanberris/md-wingman
```

You can also compare checksums with `SHA256SUMS.txt`:

```sh
shasum -a 256 -c SHA256SUMS.txt --ignore-missing
```

## Good to know

- **Keychain prompts after updates.** If you use the Jev pre-filter, macOS asks for access to your TypeSafe key in the Keychain. An ad-hoc signature changes with every build, so after you update, macOS may ask again even if you chose Always Allow before.
- Updating: download the new version and replace the app in Applications. You'll need to approve it in Privacy & Security again.
- Logs and crash reports are in `~/Library/Logs/md-wingman/`. Help ▸ Report a Problem… opens an issue.

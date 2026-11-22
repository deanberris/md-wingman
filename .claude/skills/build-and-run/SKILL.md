---
name: build-and-run
description: Build md-wingman, run it from source or install the .app on a Mac, including on Dean's iMac through Remote Control, and what can be built in a Linux container.
---

# Build and run md-wingman

md-wingman only runs on macOS (gpui uses Metal). A Linux container can build and test most of it, but not run it.

## On a Mac

From the repository root:

| Goal | Command |
|---|---|
| Run from source | `scripts/dev.sh [FILE]` (builds the service with Bun and the app in release, signs, runs) |
| Run without calling Claude | `WINGMAN_FAKE=1 scripts/dev.sh` |
| Use an API key instead of the subscription | `WINGMAN_AUTH=api_key scripts/dev.sh` (the person sets `ANTHROPIC_API_KEY`) |
| Install md-wingman.app | `scripts/install.sh` (to `/Applications`, or `~/Applications`, or `WINGMAN_INSTALL_DIR`) |
| More logging | `WINGMAN_LOG=debug`, plus `WINGMAN_LOG_TEXT=1` only if document text is needed |

- Prerequisites and signing are in AGENTS.md ("Building and installing"). Follow its rules: ask before installing tools,
  never handle the person's credentials, never strip quarantine or touch Gatekeeper.
- `dev.sh` signs with the person's "Apple Development" certificate when there is one, so the Keychain's "Always Allow"
  for the TypeSafe key survives rebuilds. Ad-hoc builds make macOS ask again after each rebuild.
- Logs: `~/Library/Logs/md-wingman/md-wingman.log`; crash reports in `~/Library/Logs/md-wingman/crashes/`. Read these
  first when something goes wrong on the Mac.
- If every task fails with "Claude didn't return a structured reply", check `claude auth status`: Claude Code is
  probably signed out. The person signs in themselves (`claude auth login`).

## Dean's iMac

Dean's clone is `~/Projects/md-wingman` on his iMac. From a cloud session, start a Remote Control session in that
folder to build, run `jev_live`, or check the app. `git pull` (or check out the PR branch) there first. Ask Dean before
doing anything that shows UI on his screen while he may be working, and leave the checkout on `main` afterwards.

## In a Linux container

```sh
apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev
cargo test --workspace --locked
```

This compiles everything except `cfg(target_os = "macos")` code and runs the headless UI tests. It can't run the app,
check Metal shaders or the macOS-only paths; CI's macOS jobs and the iMac cover those. Say which of these you could not
check when you report.

## Release build like CI

```sh
cargo build --release -p md-wingman --locked
WINGMAN_CODESIGN_IDENTITY=- WINGMAN_INSTALL_DIR="$PWD/target/Applications" scripts/install.sh
codesign --verify --strict --deep target/Applications/md-wingman.app
```

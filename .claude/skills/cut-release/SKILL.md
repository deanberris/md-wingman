---
name: cut-release
description: Prepare and cut an md-wingman release (version bump, release notes, tag-triggered workflow). Never push a version tag without Dean's explicit OK.
---

# Cut an md-wingman release

Releases are built and published by `.github/workflows/release.yml` when a `v*` tag is pushed. Pushing a tag publishes
a public download, so it can't be taken back quietly.

**Never push a version tag, or create a release, without Dean's explicit OK for that version in the conversation.**
Prepare everything up to the tag, then ask.

## 1. Prepare (a normal pull request)

1. Pick the version with Dean (semver; pre-releases look like `0.2.0-beta.1`).
2. Bump `version` under `[workspace.package]` in `Cargo.toml`, `version` in `service/package.json`, and the
   `CLAUDE_AGENT_SDK_CLIENT_APP` string in `service/src/runner.ts`. Run `cargo check --workspace` so `Cargo.lock`
   follows, then the `check` skill.
3. Read `.github/release-notes/unsigned.md` (and `signed.md`) and update anything that is no longer true. `{{VERSION}}`
   and `{{TAG}}` are filled in by the workflow.
4. Summarise what changed since the last tag (`git log $(git describe --tags --abbrev=0)..origin/main --oneline`) for
   Dean.
5. Open the PR and merge it once CI is green. If it touches packaging or the release notes, its CI also runs the release
   build without publishing.

## 2. Tag (only after Dean says yes)

```sh
git fetch origin main && git checkout origin/main
git tag -a v<version> -m "md-wingman <version>"
git push origin v<version>
```

Cloud sessions can't push tags (the git proxy refuses them), and Remote Control on Dean's iMac may stop at a permission
prompt. If neither works, Dean can publish the release on github.com instead (Releases ▸ Draft a new release, new tag
`v<version>` on `main`): that creates the tag, and the workflow fills in the release he made.

The workflow checks that the tag matches `Cargo.toml`, builds an arm64 zip and dmg, writes `SHA256SUMS.txt`, attests the
files and publishes the release.

## 3. Signing

Until the repository has the Developer ID secrets (`MACOS_DEVELOPER_ID_P12`, `..._PASSWORD`, `MACOS_NOTARY_APPLE_ID`,
`MACOS_NOTARY_PASSWORD`, `MACOS_NOTARY_TEAM_ID`), releases are ad-hoc signed and marked as unsigned pre-releases for
early adopters. With them, the same workflow signs, notarizes and staples, and publishes a normal release. Secrets are
Dean's to add; never ask for their values.

## 4. Check

Watch the run, then confirm the release page has the dmg, zip and `SHA256SUMS.txt`, and that
`gh attestation verify <file> --repo deanberris/md-wingman` passes. Report the link to Dean.

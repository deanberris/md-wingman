# Instructions for coding agents

This file is for coding agents (Claude Code, Codex, Cursor and the like)
working in this repository. It covers two jobs: building and installing
md-wingman for the person you are working with, and changing the code.

## Building and installing md-wingman on the user's Mac

The person wants md-wingman.app built from this source tree and installed on
their own Mac, so they don't have to trust a binary someone else signed. Work
through the steps in order. Don't skip a check because a later step might
cover it.

### Rules

- Only macOS is supported. If `uname` isn't `Darwin`, stop and say so.
- Ask before installing anything (Xcode tools, rustup, Bun, Claude Code), and
  say what each one is for. Don't use `sudo` without asking.
- Never ask for, read, print or store the person's passwords, Claude
  credentials or API keys. Signing in to Claude and entering a TypeSafe key
  are things they do themselves.
- Don't edit their shell profile, Keychain or settings beyond what the steps
  below say.

### 1. Check the prerequisites

Run each check and install what is missing, after asking:

| Needed | Check | Install |
|---|---|---|
| Xcode command-line tools | `xcode-select -p` | `xcode-select --install` (opens a dialog the person must accept) |
| rustup | `rustup --version` | the installer at <https://rustup.rs> |
| Bun | `bun --version` | `curl -fsSL https://bun.sh/install \| bash` |
| Claude Code | `claude --version` | <https://claude.com/claude-code> |

Notes:

- `rust-toolchain.toml` pins Rust 1.100. rustup downloads it on the first
  build; don't install a toolchain by hand. If rustup is old, run
  `rustup self update` (or `rustup update`).
- After installing rustup or Bun, a new shell may be needed before they are on
  `PATH`. Source `~/.cargo/env`, or use `~/.cargo/bin/cargo` and
  `~/.bun/bin/bun` directly.
- If the build later fails with an error about `metal`, the full Xcode is
  needed: the person installs it from the App Store, then runs
  `sudo xcode-select -s /Applications/Xcode.app`.

### 2. Make sure Claude Code is signed in

md-wingman uses the person's own Claude subscription through Claude Code.
Check with `claude auth status`. If it isn't signed in, ask the person to run
`claude auth login` (or `claude` and follow the prompts) themselves, and wait
until they say they're done. Don't try to complete the sign-in for them.

If they would rather use an Anthropic API key, that is their choice: they
launch the app with `WINGMAN_AUTH=api_key` and `ANTHROPIC_API_KEY` set. Don't
put the key anywhere yourself.

### 3. Build, sign and install

From the repository root:

```sh
scripts/install.sh
```

This builds the assistant service (Bun) and the app (cargo, release), puts
both in `md-wingman.app` (that part is `scripts/package.sh`), signs it and
copies it to `/Applications`, or
`~/Applications` if `/Applications` isn't writable. The first build takes
several minutes; let it finish.

Signing:

- If the person has an Apple Development certificate (from signing in to
  Xcode with their Apple ID, free), the script finds and uses it. Check with
  `security find-identity -v -p codesigning`.
- Otherwise it signs ad-hoc. The app still opens normally, because a locally
  built app isn't quarantined. The only cost is that macOS asks again for
  Keychain access to the TypeSafe key after each rebuild. If the person
  minds, they can sign in to Xcode (Settings ▸ Accounts) and you rerun the
  script.
- To use a specific identity, set `WINGMAN_CODESIGN_IDENTITY`.

Don't remove quarantine attributes, disable Gatekeeper or run `spctl` to get
around a signing problem. If signing fails, report the error.

### 4. Check it works

- `codesign --verify --strict --deep /Applications/md-wingman.app` (or the
  `~/Applications` path the script printed) should print nothing.
- `open /Applications/md-wingman.app` should open an empty document, with the
  status bar showing the Claude sign-in in use.
- Ask the person to put the cursor in a paragraph and press ⌘E. Suggested
  edits should appear in the sidebar. To check the plumbing without calling
  Claude, run `WINGMAN_FAKE=1 /Applications/md-wingman.app/Contents/MacOS/md-wingman`.

### 5. Optional: the Jev pre-filter

Checking as you write can use Jev, TypeSafe's scoring model, to spend Claude
requests only where they're needed. It needs a TypeSafe API key from
<https://console.typesafe.ai/keys>. Tell the person about it, but let them turn
it on in the app (Assistant ▸ Jev Pre-filter) and paste the key there. The app
stores it in their Keychain. Don't handle the key yourself.

### Updating later

`git pull`, then `scripts/install.sh` again. It replaces the installed app.

## Changing the code

- Rust workspace: `crates/md-wingman` (the gpui app), `crates/wingman-client`
  (client for the service), `crates/wingman-log` (the log file) and
  `crates/style-check` (house-style checks clippy lacks). TypeScript service in
  `service/`, on the Claude Agent SDK, compiled with Bun.
- Before you push, run what CI runs:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo test --workspace --locked` (includes the headless UI tests and the
    control-flow depth check)
  - `cd service && bun install --frozen-lockfile && bun run lint && bun run typecheck && bun test`
  - `npx markdownlint-cli2@0.24.0` and `shellcheck scripts/*.sh`
- Style: return early instead of nesting. Use `let ... else`, `?`, `return`
  and `continue` to leave the main path flat, and let chains
  (`if let Some(x) = y && x > 0`) instead of an `if` inside an `if let`.
  Control flow (`if`, `match`, loops) nests at most 3 deep in a function;
  `cargo run -p style-check` lists anything deeper. The lints are in
  `Cargo.toml` (`[workspace.lints]`) and `clippy.toml`. The service follows
  the same rule through Biome (`service/biome.json`); `bun run format` fixes
  formatting.
- Markdown wraps at 120 columns (`.markdownlint.jsonc`); tables are exempt.
  Whitespace and line endings follow `.editorconfig`.
- The UI must never interrupt the person while they type: no modal dialogs or
  Keychain prompts mid-typing. Assistant work happens in the background and
  shows up as marks by the text and in the sidebar.
- Packaging lives in `scripts/package.sh`, shared by `scripts/install.sh`
  and the release workflow (`.github/workflows/release.yml`). Pull requests
  that change either run the release build without publishing. Don't push
  version tags: releases are cut by the maintainer.
- Work is tracked in GitHub Issues.
- Log through the `log` macros. Never log keys or tokens, and never log
  document text directly: pass it through `wingman_log::text` (or `log.text`
  in the service), which shows only its size unless `WINGMAN_LOG_TEXT=1`.

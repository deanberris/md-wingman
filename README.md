# md-wingman

A native macOS Markdown editor with a Claude writing assistant beside it. The assistant fact-checks, finds references and suggests copy edits.

Work in progress. The roadmap is in [#1](https://github.com/deanberris/md-wingman/issues/1).

## Run it on a Mac

You need:

- [Rust](https://rustup.rs)
- [Bun](https://bun.sh): `curl -fsSL https://bun.sh/install | bash`
- [Claude Code](https://claude.com/claude-code), signed in. Run `claude` once and log in with your Claude account.
- Xcode command-line tools: `xcode-select --install`. If the build stops with an error about `metal`, install the full Xcode from the App Store, then run `sudo xcode-select -s /Applications/Xcode.app`.

Then:

```sh
git clone https://github.com/deanberris/md-wingman && cd md-wingman
scripts/dev.sh                  # an empty document
scripts/dev.sh notes/draft.md   # or open a file
```

The first build takes a few minutes. md-wingman uses your Claude subscription by default and ignores any `ANTHROPIC_API_KEY` in your environment. To use an API key instead, run `WINGMAN_AUTH=api_key scripts/dev.sh`. The status bar shows which one is in use.

### Keys

| Keys | Action |
|---|---|
| ⌘N / ⌘O / ⌘S / ⇧⌘S | New / Open / Save / Save As |
| ⌘P | Switch between raw Markdown and the preview |
| ⌘Q | Quit |

## Layout

- `crates/md-wingman`: the app, built with [gpui](https://gpui.rs) and [gpui-component](https://github.com/longbridge/gpui-kit)
- `crates/wingman-client`: Rust client for the assistant service
- `service/`: the assistant service, in TypeScript on the Claude Agent SDK
- `docs/`: [architecture](docs/architecture.md) and the [UI toolkit choice](docs/toolkit-choice.md)

## Try the assistant without the app

```sh
cd service && bun install && bun run build && cd ..
cargo run -p wingman-client --example wingman -- fact-check "The Eiffel Tower opened in 1899."
```

## Licence

Apache 2.0. See [LICENSE](LICENSE).

# md-wingman

A native macOS Markdown editor with a Claude writing assistant beside it. The assistant fact-checks, finds references and suggests copy edits.

Work in progress. The roadmap is in [#1](https://github.com/deanberris/md-wingman/issues/1).

## Layout

- `crates/wingman-client`: Rust client for the assistant service
- `service/`: the assistant service, in TypeScript on the Claude Agent SDK
- `docs/`: [architecture](docs/architecture.md) and the [UI toolkit choice](docs/toolkit-choice.md)

## Quick start

You need Rust, [Bun](https://bun.sh), and [Claude Code](https://claude.com/claude-code). Sign in to Claude Code with `claude`. md-wingman uses your Claude subscription by default; to use an API key instead, set `ANTHROPIC_API_KEY` and pass `--api-key` to the example.

```sh
cd service && bun install && bun run build && cd ..
cargo run -p wingman-client --example wingman -- fact-check "The Eiffel Tower opened in 1899."
```

## Licence

Apache 2.0. See [LICENSE](LICENSE).

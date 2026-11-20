# md-wingman

A native macOS Markdown editor with Claude always on hand in a sidebar, to
fact-check what you write, find references for it, and suggest copy edits.

> **Status: early development.** The assistant service and its Rust client
> work. The editor app is next. See [Building](#building) to try the
> assistant from the command line.

## What it is meant to be

md-wingman is a writing tool first. You write Markdown in a calm, minimal
editor, and an assistant sits beside the text, ready when you want a second
opinion and out of the way when you don't.

Assistant features, in priority order:

- **Copy-editing.** Select a sentence or paragraph and get suggested edits
  inline, which you accept or reject one at a time.
- **Fact-checking.** Flag claims in the selection that look doubtful, with an
  explanation and sources in the sidebar.
- **Reference lookup.** Search the web for sources that support or challenge a
  claim, and insert the one you pick as a Markdown link or footnote.

Bibliography management may come later, but it is out of scope for the first
version.

## Design

- **Native macOS app written in Rust**, with [gpui](https://gpui.rs) and
  gpui-component. The [toolkit choice](docs/toolkit-choice.md) explains why.
- **Minimal, TUI-inspired look** with careful typography.
- **Raw and preview modes.** A toggle switches between the raw Markdown and a
  rendered preview.

## Architecture

```
┌────────────────────────┐   requests (Unix socket)   ┌────────────────────────┐
│ md-wingman (Rust GUI)  │ ─────────────────────────▶ │ assistant service      │
│ editor, preview,       │                            │ Claude Agent SDK       │
│ assistant sidebar      │ ◀───────────────────────── │ TypeScript, Bun binary │
└────────────────────────┘      streamed replies      └────────────────────────┘
   starts / stops service                                  calls Claude
```

- The **Rust app** is the editor and the sidebar. It starts and stops the
  assistant service and talks to it over a local Unix socket.
- The **assistant service** is a small local background process built on the
  Claude Agent SDK. It makes all the model calls, using your own Claude
  subscription login by default, or an API key if you choose. It is written in
  TypeScript and compiled to a single executable with Bun, so it can ship
  inside the app bundle.

More detail is in [docs/architecture.md](docs/architecture.md).

## Building

The app itself isn't here yet. You can try the assistant service from the
command line. You need Rust, [Bun](https://bun.sh), and
[Claude Code](https://claude.com/claude-code), signed in with `claude`.

```sh
cd service && bun install && bun run build && cd ..
cargo run -p wingman-client --example wingman -- fact-check "The Eiffel Tower opened in 1899."
```

md-wingman uses your Claude subscription by default and ignores any
`ANTHROPIC_API_KEY` in your environment. To use an API key instead, pass
`--api-key` first: `cargo run -p wingman-client --example wingman -- --api-key fact-check "..."`.

## Layout

- `crates/wingman-client`: Rust client for the assistant service
- `service/`: the assistant service, in TypeScript on the Claude Agent SDK
- `docs/`: [architecture](docs/architecture.md) and the [UI toolkit choice](docs/toolkit-choice.md)

## Roadmap

Work is tracked in GitHub Issues:

- [#1](../../issues/1) Roadmap for the first version
- [#2](../../issues/2) Background service spike and UI toolkit choice
- [#3](../../issues/3) Minimal editor with raw/preview toggle
- [#4](../../issues/4) Copy-editing and fact-checking
- [#5](../../issues/5) Reference lookup

## Contributing

The project is at an early stage. If you are interested, the issues above
are the best place to follow along or start a discussion.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

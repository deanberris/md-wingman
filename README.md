# md-wingman

A native macOS Markdown editor with Claude always on hand in a sidebar, to
fact-check what you write, find references for it, and suggest copy edits.

> **Status: pre-alpha.** There is no working app yet. The repository is being
> set up and the first design spike is under way. Nothing here is ready to
> install or use.

## What it is meant to be

md-wingman is a writing tool first. You write Markdown in a calm, minimal
editor, and an assistant sits beside the text, ready when you want a second
opinion and out of the way when you don't.

Planned assistant features, in priority order:

- **Copy-editing.** Select a sentence or paragraph and get suggested edits
  inline, which you accept or reject one at a time.
- **Fact-checking.** Flag claims in the selection that look doubtful, with an
  explanation in the sidebar.
- **Reference lookup.** Search the web for sources that support or challenge a
  claim, and insert the one you pick as a Markdown link or footnote.

Bibliography management may come later, but it is out of scope for the first
version.

## Design

- **Native macOS app written in Rust.** The UI toolkit (gpui or iced) is still
  being chosen.
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
  Claude Agent SDK. It makes all the model calls, which lets you use your own
  Claude subscription login rather than a separate API key. The current plan is
  to write it in TypeScript and compile it to a single executable with Bun, so
  it can ship inside the app bundle.

These choices are provisional until the first spike lands.

## Roadmap

Work is tracked in GitHub Issues:

- [#1](../../issues/1) Roadmap for the first version
- [#2](../../issues/2) Background service spike and UI toolkit choice
- [#3](../../issues/3) Minimal editor with raw/preview toggle
- [#4](../../issues/4) Copy-editing and fact-checking
- [#5](../../issues/5) Reference lookup

## Building

There is nothing to build yet. Build and run instructions will be added here
once the first code lands.

## Contributing

The project is at a very early stage. If you are interested, the issues above
are the best place to follow along or start a discussion.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

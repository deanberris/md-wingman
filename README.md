# md-wingman

A native macOS Markdown editor with Claude always on hand in a sidebar, to
fact-check what you write, find references for it, and suggest copy edits.

> **Status: early development.** The editor and all three assistant features
> work, but the app has seen little use on macOS yet, so expect rough edges.

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

## Run it on a Mac

You need:

- [Rust](https://rustup.rs), installed with rustup. The repository pins the Rust version it needs (1.100), and rustup downloads it on the first build. If you already have Rust, run `rustup update` first so rustup itself is current.
- [Bun](https://bun.sh): `curl -fsSL https://bun.sh/install | bash`
- [Claude Code](https://claude.com/claude-code), signed in. Run `claude` once and log in with your Claude account.
- Xcode command-line tools: `xcode-select --install`. If the build stops with an error about `metal`, install the full Xcode from the App Store, then run `sudo xcode-select -s /Applications/Xcode.app`.

Then:

```sh
git clone https://github.com/deanberris/md-wingman && cd md-wingman
scripts/dev.sh                  # an empty document
scripts/dev.sh notes/draft.md   # or open a file
```

The first time it opens, md-wingman shows a short setup in the Assistant pane. It checks that Claude Code is installed and signed in (`claude auth status`), and if not, gives you the command to install it or to sign in (`claude auth login`). Copy a command into Terminal yourself, or press **Run in Terminal** and md-wingman opens a Terminal window running it. It checks again every few seconds, so it notices once you've signed in. The second step, setting up the Jev pre-filter, is optional and can be skipped. Setup never takes the focus from the editor, so you can start writing straight away. It comes back at the next launch until Claude Code is signed in, and you can reopen it any time from Assistant ▸ Setup….

The first build takes a few minutes. md-wingman uses your Claude subscription by default and ignores any `ANTHROPIC_API_KEY` in your environment. To use an API key instead, run `WINGMAN_AUTH=api_key scripts/dev.sh`. The status bar shows which one is in use.

### Using the assistant

Select a passage, or just put the cursor in a paragraph, then press ⌘E to copy-edit, ⇧⌘C to fact-check or ⇧⌘R to find references (the buttons at the top of the sidebar do the same). Suggested edits are underlined in the text and listed in the sidebar, where you can accept or dismiss each one, or accept them all. Fact-checked claims are shaded by verdict and come with sources. Each reference can be cited right after the claim it backs, as a footnote (with the full citation added at the end of the document) or as an inline link. Click a card to jump to its text. Set `WINGMAN_FAKE=1` to try the flow with canned replies, without calling Claude.

Findings stay with the text they are about. Edit around them and they follow it; delete the text and they disappear, and undo brings them back. Accepting edits, alone or all at once, is a single step that ⌘Z takes back, suggestions included.

Fact-checks are kept after you close their card. They are listed under "Fact-checks in this document", show up under "At the cursor" when you are in the claim's text, and are saved, so they are still there when you reopen the file. They are stored outside the document, in `~/Library/Application Support/md-wingman/fact-checks/`.

### Checking as you write

Turn on **As you write** (⇧⌘K, or the button at the top of the sidebar) and md-wingman copy-edits and fact-checks paragraphs on its own. It waits until you stop typing for a moment (3 seconds by default), so a burst of dictation is checked once rather than word by word. Then it checks one paragraph at a time and shows what it finds under "Suggestions" and "Fact-checks in this document".

To keep requests down:

- Only new or changed paragraphs are checked. What was already in the document when you turned it on or opened the file counts as checked, and the sidebar buttons check those paragraphs when you ask.
- The paragraph you are writing is checked once it ends a sentence.
- Headings, code, tables and short lines are skipped, and so is a blog post's front matter.
- If you change a paragraph while it is being checked, that check is cancelled and redone after your next pause.
- Accepting a suggestion doesn't count as a change that needs checking again.

### Jev pre-filter

With [Jev](https://docs.typesafe.ai), TypeSafe's scoring model, checking as you write spends Claude requests only where they are needed. Before checking a paragraph, md-wingman asks Jev to grade how much it needs a copy-edit, a fact-check and references. It acts only on grades above 66%, and that includes finding references, which it doesn't do without Jev. Grading a paragraph costs a fraction of a cent. The status bar shows the latest grades.

Turn it on with the **Jev pre-filter** button in the sidebar, or from Assistant ▸ Jev Pre-filter. The first time, it asks for your TypeSafe API key ([console.typesafe.ai/keys](https://console.typesafe.ai/keys)). The key is stored in your macOS Keychain under "md-wingman", never in the settings file. You can also save it from Terminal with `security add-generic-password -U -s md-wingman -a typesafe-api-key -w`, which asks for the key so it stays out of your shell history; the first time md-wingman reads it, macOS asks whether to allow that. To change or remove it, use Assistant ▸ Set TypeSafe API Key…. If TypeSafe rejects the key, checking pauses until you set a new one. If Jev is busy, it waits and tries again.

md-wingman reads the key from the Keychain once per launch and keeps it in memory. When you build from source, macOS may still ask on every launch, because each rebuild gives the app a new code signature and macOS ties "Always Allow" to the signature. To make it stick, sign your builds with an Apple Development certificate:

1. In Xcode, open Settings ▸ Accounts and sign in with your Apple ID. A free account is enough. Under Manage Certificates…, add an Apple Development certificate if there isn't one.
2. Run `scripts/dev.sh` as usual. It signs the build with your Apple Development certificate (or with `WINGMAN_CODESIGN_IDENTITY`). Choose Always Allow at the next Keychain prompt, and later rebuilds won't ask again.

Ad-hoc signing (`codesign -s -`) doesn't help, because an ad-hoc signature also changes with every build.

### Reviewing the whole document

Assistant ▸ Review Whole Document (⇧⌘D) copy-edits, fact-checks and finds references for the whole post at once, in the background, while you keep writing. Its findings are anchored to the text like any others and listed under "Suggestions" and "Fact-checks in this document". A long post is reviewed in parts, split between paragraphs: each part goes with the whole post as context when that fits, or else with the post's headings and the text just before and after it. A paragraph you change while the review runs is reviewed again as it is now, and paragraphs it has reviewed aren't checked again as you write.

It never starts by itself. With checking as you write and the Jev pre-filter on, Jev also grades now and then whether the draft looks ready for a full look-over: a complete draft, its structure settled. It asks only after you pause for 30 seconds, only once a post has a few paragraphs, and again only after the post has changed. Above 66%, the sidebar suggests a review, with a button to start it and **Not now** to put it away until you've written more.

### Appearance and fonts

Click **Aa** at the top of the sidebar, or choose View ▸ Appearance and Fonts…, to pick light, dark or system mode (which follows macOS), the editor's monospace font and size, and the preview's reading font. Only fonts installed on your Mac are offered. View ▸ Toggle Dark Mode switches modes, and ⌘= / ⌘− / ⌘0 change the editor's text size. The ruler stays at its column whatever the font.

### Keys

| Keys | Action |
|---|---|
| ⌘N / ⌘O / ⌘S / ⇧⌘S | New / Open / Save / Save As |
| ⌘P | Switch between raw Markdown and the preview |
| ⌘E | Copy-edit the selection, or the paragraph under the cursor |
| ⇧⌘C | Fact-check the selection, or the paragraph under the cursor |
| ⇧⌘R | Find references for the selection, or the paragraph under the cursor |
| ⇧⌘D | Review the whole document |
| ⇧⌘K | Check as you write, on or off |
| ⌘= / ⌘− / ⌘0 | Bigger / smaller / default editor text |
| ⌥⌘Q | Reflow the paragraph (or selection) to the ruler column |
| ⌘Z / ⇧⌘Z | Undo / Redo |
| ⌘Q | Quit |

### Settings

Settings live in `~/Library/Application Support/md-wingman/settings.json`. The
app writes the file when you change something from the menus or drag the
divider, and you can edit it by hand. Missing fields take their defaults.

```json
{
  "ruler_column": 120,
  "show_ruler": true,
  "wrap_at_ruler": true,
  "assistant_fraction": 0.382,
  "continuous": false,
  "continuous_delay_ms": 3000,
  "continuous_fact_check": true,
  "jev_prefilter": false,
  "jev_threshold": 0.66,
  "jev_model": "jev-1.14.0",
  "appearance": "system",
  "editor_font": null,
  "editor_font_size": 14,
  "preview_font": null,
  "setup_done": false
}
```

`wrap_at_ruler` soft-wraps text at the ruler instead of at the window edge.
`assistant_fraction` is the assistant pane's share of the window width. The
default splits the window at the golden ratio, and dragging the divider
changes it. `continuous` turns on checking as you write.
`continuous_delay_ms` is how long typing must pause before a check. Set
`continuous_fact_check` to `false` to copy-edit only, since fact-checks search
the web and take longer. `jev_threshold` is the grade, from 0 to 1, that a
paragraph must score above for the pre-filter to act on it. `appearance` is
`"system"`, `"light"` or `"dark"`. `setup_done` is set once you finish setup with Claude Code signed in, so setup stops opening at launch. A `null` font means the default: Menlo for
the editor and the system font for the preview.

### Try the assistant without the app

```sh
cd service && bun install && bun run build && cd ..
cargo run -p wingman-client --example wingman -- fact-check "The Eiffel Tower opened in 1899."
```

## Tests

```sh
cargo test --workspace          # unit tests and the headless UI tests
cd service && bun test          # the assistant service
```

The UI tests (`crates/md-wingman/src/ui_tests/`) open the real window
headlessly with gpui's test harness and drive it with clicks, typing and
shortcuts. A scripted assistant stands in for the service and a fake Jev for
TypeSafe, so they need no Claude login, TypeSafe key or Keychain, and the
pause before checking as you write runs on the test clock. CI runs all of it
on macOS for every pull request.

## Layout

- `crates/md-wingman`: the app, built with [gpui](https://gpui.rs) and [gpui-component](https://github.com/longbridge/gpui-component)
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

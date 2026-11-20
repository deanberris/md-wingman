# Architecture

md-wingman has two processes:

```
┌──────────────────────────┐   Unix socket, NDJSON   ┌─────────────────────────────┐
│ md-wingman (Rust GUI)    │ ──────────────────────▶ │ wingman-service (TypeScript)│
│ editor, preview, sidebar │ ◀────────────────────── │ Claude Agent SDK            │
│ crates/wingman-client    │   streamed events       │ service/                    │
└──────────────────────────┘                         └──────────────┬──────────────┘
                                                                    │ runs
                                                                    ▼
                                                           Claude Code (`claude`)
```

- **The app** is written in Rust. It owns the documents and the UI. It never calls Claude directly.
- **The service** is a small TypeScript program built on the Claude Agent SDK. It makes every model call. Bun compiles it into a single executable, which ships with the app.
- **`crates/wingman-client`** is the Rust side of the link. It starts the service, connects to it and streams replies. It uses plain threads and channels, so it works with any UI toolkit.

## Lifecycle

The app starts the service with `--socket <path>` and holds the service's stdin open. The service exits when:
- the app drops its `Service` handle (stdin closes),
- the app crashes or is killed (stdin closes too, so no orphaned process is left),
- a client sends `shutdown`, or
- it gets SIGTERM or SIGINT.

The socket is created with mode 0600 in the user's temp directory, so only the same user can connect.

## Protocol (version 1)

Each message is one JSON object per line. Every request carries a client-chosen `id`, and every event the service sends back carries the `id` of the request it answers. Several requests can be in flight on one connection at once.

Requests (app → service):

| type | fields | meaning |
|---|---|---|
| `hello` | | Version handshake |
| `task` | `task`, `text`, `context?` | Start an assistant task on `text`. `task` is `copy_edit`, `fact_check` or `find_references` |
| `cancel` | `target` | Stop the task whose id is `target` |
| `shutdown` | | Exit the service |

Events (service → app):

| type | fields | meaning |
|---|---|---|
| `hello` | `protocol`, `version` | Reply to `hello` |
| `delta` | `text` | Next piece of the reply |
| `status` | `message` | Progress, such as "Using WebSearch" |
| `done` | `text` | Finished. `text` is the whole reply |
| `error` | `message` | Failed or was cancelled. `message` is `cancelled` for a cancel |

Every task ends with exactly one `done` or `error`. The types live in `service/src/protocol.ts` and `crates/wingman-client/src/protocol.rs`, and the two files must stay in sync.

In this version, replies are free Markdown text. Copy-editing (#4) will add structured suggestions that the editor can apply inline.

## Tasks

| task | tools | max turns |
|---|---|---|
| `copy_edit` | none | 1 |
| `fact_check` | WebSearch, WebFetch | 12 |
| `find_references` | WebSearch, WebFetch | 12 |

The service runs Claude Code with these restrictions:
- Each task can use only the tools listed for it, and any other tool call is refused, never prompted for (`permissionMode: "dontAsk"`).
- The working directory is an empty temp directory.
- The user's own CLAUDE.md files and settings are not loaded.
- Sessions are not saved.

Prompts are in `service/src/tasks.ts`.

## Authentication

The service uses whatever authentication Claude Code uses on the machine:
- **A Claude subscription:** run `claude` once and sign in.
- **An API key:** set `ANTHROPIC_API_KEY` in the environment the app starts with.

Anthropic's Agent SDK documentation says: *"Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK."* An API key is the supported route for an app that other people download. See the open decision on #2.

## Finding Claude Code

Run from source (`bun run src/main.ts`), the SDK uses the copy of Claude Code it installed in `node_modules`. The compiled executable can't see `node_modules`, so `service/src/claudePath.ts` looks for Claude Code in this order:
1. `WINGMAN_CLAUDE_PATH`
2. `claude` on `PATH`
3. `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin` and `/usr/local/bin`

An app started from Finder doesn't get the shell's `PATH`, which is why the fixed locations are checked.

The SDK can also embed Claude Code inside the compiled service (see the SDK README, "Compiled binaries"). That avoids needing a separate install, but the embedded executable is about 380 MB. Packaging will decide this later.

## UI toolkit

See [toolkit-choice.md](toolkit-choice.md).

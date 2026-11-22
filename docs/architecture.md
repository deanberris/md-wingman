# Architecture

md-wingman has two processes:

```text
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
- **The service** is a small TypeScript program built on the Claude Agent SDK. It makes every model call. Bun compiles
  it into a single executable, which ships with the app.
- **`crates/wingman-client`** is the Rust side of the link. It starts the service, connects to it and streams replies.
  It uses plain threads and channels, so it works with any UI toolkit.

## Lifecycle

The app starts the service with `--socket <path>` and holds the service's stdin open. The service exits when:

- the app drops its `Service` handle (stdin closes),
- the app crashes or is killed (stdin closes too, so no orphaned process is left),
- a client sends `shutdown`, or
- it gets SIGTERM or SIGINT.

The socket is created with mode 0600 in the user's temp directory, so only the same user can connect.

## Protocol (version 1)

Each message is one JSON object per line. Every request carries a client-chosen `id`, and every event the service sends
back carries the `id` of the request it answers. Several requests can be in flight on one connection at once.

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

Every task ends with exactly one `done` or `error`. The types live in `service/src/protocol.ts` and
`crates/wingman-client/src/protocol.rs`, and the two files must stay in sync.

In this version, replies are free Markdown text. Copy-editing (#4) will add structured suggestions that the editor can
apply inline.

## Tasks

| task | tools | max turns |
|---|---|---|
| `copy_edit` | none | 1 |
| `fact_check` | WebSearch, WebFetch | 12 |
| `find_references` | WebSearch, WebFetch | 12 |

The service runs Claude Code with these restrictions:

- Each task can use only the tools listed for it, and any other tool call is refused, never prompted for
  (`permissionMode: "dontAsk"`).
- The working directory is an empty temp directory.
- The user's own CLAUDE.md files and settings are not loaded.
- Sessions are not saved.

Prompts are in `service/src/tasks.ts`.

## Authentication

The service never handles credentials itself. Claude Code, run through the official Agent SDK, does the signing in. The
service only decides which credential Claude Code is allowed to see. `WINGMAN_AUTH` sets the mode; the Rust side passes
it as `AuthMode` to `Service::spawn`:

| mode | how to sign in | billed to | removed from Claude Code's environment |
|---|---|---|---|
| `subscription` (default) | run `claude` once and sign in, or set `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token` | your Claude plan's usage limits | `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_FOUNDRY_API_KEY` |
| `api_key` | set `ANTHROPIC_API_KEY` | the Claude API | `CLAUDE_CODE_OAUTH_TOKEN` |
| `provider` | the provider's token, if any (Assistant ▸ Model Provider…, kept in the Keychain) | the provider | all of the above, and `CLAUDE_CODE_USE_BEDROCK`/`_VERTEX`/`_FOUNDRY` |

In subscription mode, a stray API key in the environment can't quietly switch billing to the API. In `api_key` mode, the
service refuses to start without a key. The `hello` reply reports the mode, so the app can show which one is in use.

### Other model providers

In `provider` mode (`service/src/auth.ts`), Claude Code talks to another endpoint that speaks Anthropic's Messages API,
such as OpenRouter, Ollama or LM Studio. The app passes the provider from its settings
(`crates/md-wingman/src/provider.rs`) to the service as `WINGMAN_PROVIDER_URL`, `WINGMAN_PROVIDER_NAME`,
`WINGMAN_MODEL`, `WINGMAN_FAST_MODEL`, `WINGMAN_WEB_SEARCH` and, if there is one, `WINGMAN_PROVIDER_TOKEN`, read from
the Keychain at launch or when the provider changes. The service turns them into Claude Code's own variables:

| Claude Code variable | value |
|---|---|
| `ANTHROPIC_BASE_URL` | the provider's URL |
| `ANTHROPIC_AUTH_TOKEN` | the token, or a placeholder when there is none |
| `ANTHROPIC_API_KEY` | empty |
| `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_OPUS_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL` | the model |
| `ANTHROPIC_DEFAULT_HAIKU_MODEL` | the fast model, or the model |

The token is always set, even to a placeholder, and the API key is set but empty, so Claude Code never falls back to the
user's Claude login and sends it to someone else's server. With web search off, the WebSearch tool is left out of the
fact-check and references tasks and their instructions say to work from what the model knows. The `hello` reply carries
the provider's name and model for the status bar. Changing the provider restarts the service, cancelling any runs in
flight.

Anthropic's support article ["Use the Claude Agent SDK with your Claude
plan"](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) says Agent SDK
usage counts against your plan's limits when you're signed in with your plan. The Agent SDK overview also says: *"Unless
previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their
products, including agents built on the Claude Agent SDK."* md-wingman doesn't offer a claude.ai login; users sign in to
Claude Code themselves. Before a public release, check whether that wording applies to a downloaded app that defaults to
subscription mode.

## Finding Claude Code

Run from source (`bun run src/main.ts`), the SDK uses the copy of Claude Code it installed in `node_modules`. The
compiled executable can't see `node_modules`, so `service/src/claudePath.ts` looks for Claude Code in this order:

1. `WINGMAN_CLAUDE_PATH`
2. `claude` on `PATH`
3. `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin` and `/usr/local/bin`

An app started from Finder doesn't get the shell's `PATH`, which is why the fixed locations are checked.

The SDK can also embed Claude Code inside the compiled service (see the SDK README, "Compiled binaries"). That avoids
needing a separate install, but the embedded executable is about 380 MB. Packaging will decide this later.

## UI toolkit

See [toolkit-choice.md](toolkit-choice.md).

# wingman-service

The assistant back-end for md-wingman. It is a small TypeScript program on the [Claude Agent SDK](https://code.claude.com/docs/en/agent-sdk/overview). It listens on a Unix socket and streams Claude's replies to the app. See [../docs/architecture.md](../docs/architecture.md) for the protocol.

## Develop

```sh
bun install
bun test             # protocol and server tests, with canned replies
bun run typecheck
bun run start -- --socket /tmp/wingman.sock   # run from source
bun run build        # single executable at dist/wingman-service
```

## Environment

| variable | effect |
|---|---|
| `WINGMAN_FAKE=1` | Answer with canned replies instead of calling Claude (for tests and UI work) |
| `WINGMAN_MODEL` | Claude model to use. Defaults to Claude Code's default |
| `WINGMAN_CLAUDE_PATH` | Claude Code executable to run (see "Finding Claude Code" in the architecture doc) |
| `WINGMAN_AUTH` | `subscription` (default): use your `claude` login and ignore API keys. `api_key`: use `ANTHROPIC_API_KEY` and bill the Claude API |
| `ANTHROPIC_API_KEY` | The key used in `api_key` mode. Ignored in subscription mode |

## Try it from Rust

```sh
bun run build
cd .. && cargo run -p wingman-client --example wingman -- copy-edit "Their going to the libary."
```

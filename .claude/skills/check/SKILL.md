---
name: check
description: Run md-wingman's CI checks locally (fmt, clippy, tests, style-check, Biome, markdownlint, shellcheck) before pushing or after a change.
---

# Run the checks CI runs

CI (`.github/workflows/ci.yml`) runs three jobs: `lint` on Linux, and `app` and `service` on an Apple Silicon Mac.
Run the same commands before you push. Run only what the change touches while you iterate, and the lot once before
pushing.

## Rust

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

- `cargo test` includes the headless UI tests and `crates/style-check`, which fails when `if`/`match`/loops nest more
  than 3 deep in a function. `cargo run -p style-check` lists every place that does.
- To fix a nesting failure, return early (`let ... else`, `?`, `return`, `continue`), use a let chain
  (`if let Some(x) = y && x.ok()`), or move the inner part into a function. Don't raise the limits in `clippy.toml` or
  `style-check` to make a change pass.
- On Linux (a cloud container), install `libxkbcommon-dev libxkbcommon-x11-dev` with apt first. macOS-only code
  (`cfg(target_os = "macos")`, Metal) is only compiled by CI or on the Mac; see the `build-and-run` skill.
- To re-run one failing test: `cargo test -p md-wingman <name>`.

## Service

```sh
cd service
bun install --frozen-lockfile
bun run lint        # Biome lint + format check; `bun run format` fixes formatting
bun run typecheck
bun test
```

## Docs, whitespace and scripts

```sh
npx --yes markdownlint-cli2@0.24.0   # Markdown wraps at 120 columns; tables are exempt
shellcheck scripts/*.sh
```

Whitespace and line endings follow `.editorconfig`; CI checks them with editorconfig-checker.

## When something fails

Fix the cause, then re-run only what failed. Never skip, ignore or `#[ignore]` a test, and never add `allow(...)` or
`biome-ignore` to get a check green without saying why in a comment and in the PR.

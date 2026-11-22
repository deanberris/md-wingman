---
name: add-tests
description: Add tests to md-wingman, especially headless UI tests with the gpui-kit harness, FakeAssistant and FakeJev, plus service tests and the shared contract fixtures.
---

# Add tests to md-wingman

Every behaviour change gets a test, and every bug fix gets one that fails without the fix. Run them with
`cargo test -p md-wingman <filter>` and `cd service && bun test`.

## Headless UI tests (Rust)

They live in `crates/md-wingman/src/ui_tests/`, one module per area (`assistant`, `continuous`, `findings`, `review`,
`setup`, `quit`, `about`, `view`, `contract`). They run the real `Workspace` in a gpui test window with fakes in place
of the service, TypeSafe, the Keychain and the settings file, so they need no login or network.

Shape of a test:

```rust
use gpui_kit::TestAppContext;

use super::*;

#[gpui_kit::test]
fn accepting_a_suggestion_fixes_the_text(cx: &mut TestAppContext) {
    let h = open(cx, Setup { text: "Their are three towers.", ..Default::default() });
    h.cursor_at(cx, "three");
    h.click(cx, "run-copy-edit");
    h.assistant.send(0, copy_edit_reply(&[("Their are", "There are")]));
    cx.run_until_parked();
    let id = h.finding_id(cx, "Their are");
    h.click(cx, finding_button("accept", "run", id));
    assert_eq!(h.text(cx), "There are three towers.");
}
```

- `open(cx, Setup { .. })` builds the window. `Setup` picks the text, settings, a first-run setup state, the Claude
  status and the fakes. Look at the struct in `ui_tests/mod.rs` before adding fields.
- Drive it like a person: `h.click(id)`, `h.press(cx, "cmd-e")` (use `shortcut("e")`, `UNDO` and `REDO` so tests pass on
  Linux too), `h.type_text`, `h.cursor_at`, `h.select`. Read back with `h.text`, `h.notice`, `h.open_findings`,
  `h.exists(id)`.
- `h.assistant` is the `FakeAssistant`: `requests()`, `context(i)`, `was_cancelled(i)`, `send(i, events)` and
  `answer_with(f)`. Build replies with `done(...)`, `copy_edit_reply(...)` and the other helpers in `mod.rs`.
- `FakeJev` grades with `grade_with`/`queue`, checks readiness with `readiness_with`, and can `deny_keychain`.
- Time: continuous mode reads `cx.background_executor().now()`, so `h.wait(cx, ms)` / `advance_clock` drives debounces.
  Call `cx.run_until_parked()` after sending events.
- Only Buttons, and elements with `.test_support()` after `.id(...)`, can be found by id. Plain `div().id(..)` can't.
- Don't `use gpui_kit::*` in a test module: its glob shadows `#[test]`.
- Prompts (save on quit): `cx.pending_prompt()`, `simulate_prompt_answer`, `simulate_new_path_selection`.
- Check the no-interrupt rule where it applies: background work must not move the cursor or focus, and must not read
  the key (`FakeJev::key_reads`) mid-typing.

## Unit tests

Pure logic (ranges, reflow, verdicts, parsing) gets a `#[cfg(test)] mod tests` next to the code. Prefer a unit test when
no UI is involved.

## Service tests (TypeScript)

`service/src/*.test.ts` with `bun:test`. `makeAgentRunner(auth, { query })` takes a scripted SDK `query`, so runner
tests never call Claude. `server.test.ts` talks to a real socket with a fake runner.

## Contract fixtures

`service/fixtures/<task>.json` hold one structured reply per task. `service/src/contract.test.ts` checks them against the
task schemas and runs them through the runner and server; `ui_tests/contract.rs` parses the same files in the app. When
a schema changes, update the fixture and both tests together.

## Tests that call real services

Only the `jev_live` calibration, which is `#[ignore]`d. See the `tune-jev` skill. Never make a normal test depend on
the network, a login or the Keychain.

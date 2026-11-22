# Fixtures

Structured replies, one per task, shared by the service's and the app's
tests. `src/contract.test.ts` checks each one against its task's JSON schema
and sends it through the real runner and server; the app's UI tests
(`crates/md-wingman/src/ui_tests/contract.rs`) read the same files and check
that every item shows up against its passage. Change them together with the
schemas in `src/tasks.ts`.

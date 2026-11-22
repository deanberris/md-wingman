import { expect, test } from "bun:test";
import { friendlyError, NOT_SIGNED_IN, resultSummary, taskResult } from "./runner";
import { COPY_EDIT_SCHEMA } from "./tasks";

const schema = { schema: COPY_EDIT_SCHEMA as Record<string, unknown> };

test("a signed-out Claude Code says how to sign in, not that the structured reply is missing", () => {
  // What the SDK yields when Claude Code isn't logged in.
  const msg = { subtype: "success", is_error: true, result: "Not logged in · Please run /login" } as const;
  expect(() => taskResult(msg, schema)).toThrow(NOT_SIGNED_IN);
  expect(NOT_SIGNED_IN).toContain("claude auth login");
});

test("other API errors pass through unchanged", () => {
  expect(friendlyError("API Error: 529 Overloaded")).toBe("API Error: 529 Overloaded");
  expect(() => taskResult({ subtype: "success", is_error: true, result: "API Error: 529 Overloaded" }, schema)).toThrow(
    "API Error: 529 Overloaded",
  );
});

test("an error result without text still fails clearly", () => {
  expect(() => taskResult({ subtype: "success", is_error: true, result: "" }, schema)).toThrow(
    "Claude returned an error",
  );
});

test("a structured reply is returned with its summary", () => {
  const data = { summary: "One fix.", edits: [{ original: "teh", replacement: "the", reason: "typo" }] };
  expect(taskResult({ subtype: "success", is_error: false, result: "", structured_output: data }, schema)).toEqual({
    text: "One fix.",
    data,
  });
});

test("a missing structured reply is still reported", () => {
  expect(() => taskResult({ subtype: "success", is_error: false, result: "hi" }, schema)).toThrow(
    "Claude didn't return a structured reply",
  );
});

test("tasks without a schema return the text", () => {
  expect(taskResult({ subtype: "success", is_error: false, result: "" }, {}, "streamed")).toEqual({ text: "streamed" });
});

test("early stops name the reason", () => {
  expect(() => taskResult({ subtype: "error_max_turns", is_error: true }, schema)).toThrow(
    "Claude stopped early (error_max_turns)",
  );
});

test("result summaries carry time, turns, cost and tokens, not text", () => {
  const line = resultSummary({
    subtype: "success",
    is_error: false,
    duration_ms: 4200,
    duration_api_ms: 3900,
    num_turns: 3,
    total_cost_usd: 0.01234,
    usage: {
      input_tokens: 10,
      output_tokens: 20,
      cache_read_input_tokens: 30,
      cache_creation_input_tokens: 40,
    } as never,
    result: "the reply text",
  });
  expect(line).toBe(
    "result success in 4200ms (API 3900ms), 3 turns, $0.0123, tokens in 10, out 20, cache read 30, cache write 40",
  );
  expect(line).not.toContain("reply text");
});

// The contract between Claude's structured replies, the service and the app.
//
// The fixtures in ../fixtures are what the app expects a task to return. Here
// each one is checked against its task's JSON schema, then sent through the
// real runner and server, with the Agent SDK's `query` replaced by a script,
// to check what reaches the socket. The app's UI tests read the same files.

import { afterEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import type { Event, TaskKind } from "./protocol";
import { fakeRunner, makeAgentRunner, type QueryFn } from "./runner";
import { startServer } from "./server";
import { TASKS } from "./tasks";

const KINDS: TaskKind[] = ["copy_edit", "fact_check", "find_references"];

interface Fixture {
  passage: string;
  data: Record<string, unknown> & { summary: string };
}

function fixture(kind: TaskKind): Fixture {
  return JSON.parse(readFileSync(join(import.meta.dir, "..", "fixtures", `${kind}.json`), "utf8"));
}

/** Checks `value` against the subset of JSON Schema the task schemas use. */
function violations(schema: Record<string, any>, value: unknown, path = "$"): string[] {
  switch (schema.type) {
    case "object": {
      if (typeof value !== "object" || value === null || Array.isArray(value)) return [`${path}: not an object`];
      const obj = value as Record<string, unknown>;
      const props: Record<string, any> = schema.properties ?? {};
      const out: string[] = [];
      for (const key of schema.required ?? []) if (!(key in obj)) out.push(`${path}.${key}: missing`);
      for (const [key, v] of Object.entries(obj)) {
        if (props[key]) out.push(...violations(props[key], v, `${path}.${key}`));
        else if (schema.additionalProperties === false) out.push(`${path}.${key}: not allowed`);
      }
      return out;
    }
    case "array":
      if (!Array.isArray(value)) return [`${path}: not an array`];
      return value.flatMap((v, i) => violations(schema.items, v, `${path}[${i}]`));
    case "string":
      if (typeof value !== "string") return [`${path}: not a string`];
      if (schema.enum && !schema.enum.includes(value)) return [`${path}: ${value} not in ${schema.enum}`];
      return [];
    default:
      return [`${path}: unknown schema type ${schema.type}`];
  }
}

/** The quotes a reply's items point at in the passage. */
function quotes(kind: TaskKind, data: Record<string, any>): string[] {
  if (kind === "copy_edit") return data.edits.map((e: { original: string }) => e.original);
  if (kind === "fact_check") return data.claims.map((c: { quote: string }) => c.quote);
  return data.references.map((r: { quote: string }) => r.quote);
}

for (const kind of KINDS) {
  test(`the ${kind} fixture matches its schema and quotes its passage`, () => {
    const { passage, data } = fixture(kind);
    expect(violations(TASKS[kind].schema!, data)).toEqual([]);
    for (const quote of quotes(kind, data)) expect(passage).toContain(quote);
  });

  test(`WINGMAN_FAKE's ${kind} reply matches its schema`, async () => {
    const { passage } = fixture(kind);
    const reply = await fakeRunner({
      task: kind,
      text: passage,
      signal: new AbortController().signal,
      onDelta: () => {},
      onStatus: () => {},
    });
    expect(violations(TASKS[kind].schema!, reply.data)).toEqual([]);
  });
}

test("the checker catches a reply of the wrong shape", () => {
  const schema = TASKS.fact_check.schema!;
  expect(violations(schema, { summary: "x" })).toEqual(["$.claims: missing"]);
  expect(violations(schema, { summary: "x", claims: [{ quote: "q", verdict: "maybe", explanation: "", sources: [] }] })).toEqual([
    "$.claims[0].verdict: maybe not in supported,disputed,false,unverifiable",
  ]);
});

// Through the runner and the server.

let stop: (() => void) | undefined;
afterEach(() => stop?.());

type QueryArgs = Parameters<QueryFn>[0];

/** A `query` that records its arguments and yields `messages`. */
function scripted(messages: unknown[]): { query: QueryFn; calls: QueryArgs[] } {
  const calls: QueryArgs[] = [];
  return {
    calls,
    query: (args) => {
      calls.push(args);
      return (async function* () {
        for (const m of messages) yield m as SDKMessage;
      })();
    },
  };
}

const stream = (event: unknown) => ({ type: "stream_event", parent_tool_use_id: null, event });
const result = (fields: Record<string, unknown>) => ({ type: "result", subtype: "success", is_error: false, result: "", ...fields });

/** Runs one task through a server on `query` and returns its events. */
async function runTask(query: QueryFn, kind: TaskKind, text: string, context?: string): Promise<Event[]> {
  const socketPath = join(tmpdir(), `wingman-contract-${process.pid}-${Math.random().toString(36).slice(2)}.sock`);
  stop = startServer({ socketPath, runner: makeAgentRunner("subscription", { query }) }).stop;
  const events: Event[] = [];
  let buf = "";
  let finished!: () => void;
  const done = new Promise<void>((r) => (finished = r));
  const sock = await Bun.connect({
    unix: socketPath,
    socket: {
      data(_s, chunk) {
        buf += chunk.toString();
        let nl: number;
        while ((nl = buf.indexOf("\n")) >= 0) {
          const ev = JSON.parse(buf.slice(0, nl)) as Event;
          buf = buf.slice(nl + 1);
          events.push(ev);
          if (ev.type === "done" || ev.type === "error") finished();
        }
      },
    },
  });
  sock.write(JSON.stringify({ id: "t", type: "task", task: kind, text, context }) + "\n");
  await done;
  sock.end();
  return events;
}

for (const kind of KINDS) {
  test(`a ${kind} structured reply reaches the app as done.data`, async () => {
    const { passage, data } = fixture(kind);
    const sdk = scripted([
      stream({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Looking…" } }),
      stream({ type: "content_block_start", index: 1, content_block: { type: "tool_use", id: "s", name: "StructuredOutput", input: {} } }),
      result({ structured_output: data }),
    ]);
    const events = await runTask(sdk.query, kind, passage, `# Post\n\n${passage}`);

    expect(events.map((e) => e.type)).toEqual(["delta", "status", "done"]);
    expect(events[1]).toMatchObject({ type: "status", message: "Writing up" });
    expect(events[2]).toEqual({ id: "t", type: "done", text: data.summary, data });

    // The SDK was asked for this task's schema, with the passage and context in the prompt.
    const options = sdk.calls[0]!.options!;
    expect(options.outputFormat).toEqual({ type: "json_schema", schema: TASKS[kind].schema! });
    expect(options.permissionMode).toBe("dontAsk");
    expect(sdk.calls[0]!.prompt).toContain(`<passage>\n${passage}\n</passage>`);
    expect(sdk.calls[0]!.prompt).toContain("<document_context>");
  });
}

test("a signed-out Claude Code reaches the app as its own error", async () => {
  const sdk = scripted([result({ is_error: true, result: "Not logged in · Please run /login" })]);
  const events = await runTask(sdk.query, "copy_edit", "teh cat");
  expect(events.at(-1)).toEqual({ id: "t", type: "error", message: "Not logged in · Please run /login" });
});

test("a result without the structured reply is an error, not an empty done", async () => {
  const sdk = scripted([result({ result: "Here are some edits." })]);
  const events = await runTask(sdk.query, "copy_edit", "teh cat");
  expect(events.at(-1)).toEqual({ id: "t", type: "error", message: "Claude didn't return a structured reply" });
});

test("a run that stops early names why", async () => {
  const sdk = scripted([{ type: "result", subtype: "error_max_turns", is_error: true }]);
  const events = await runTask(sdk.query, "fact_check", "The moon is cheese.");
  expect(events.at(-1)).toEqual({ id: "t", type: "error", message: "Claude stopped early (error_max_turns)" });
});

test("tool use other than the reply shows as status", async () => {
  const { passage, data } = fixture("fact_check");
  const sdk = scripted([
    stream({ type: "content_block_start", index: 0, content_block: { type: "server_tool_use", id: "w", name: "web_search", input: {} } }),
    result({ structured_output: data }),
  ]);
  const events = await runTask(sdk.query, "fact_check", passage);
  expect(events[0]).toMatchObject({ type: "status", message: "Using web_search" });
});

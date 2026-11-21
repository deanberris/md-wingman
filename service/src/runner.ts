import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { query, type SDKMessage, type SDKResultMessage } from "@anthropic-ai/claude-agent-sdk";
import { claudeEnv, type AuthMode } from "./auth";
import { resolveClaudePath } from "./claudePath";
import type { TaskKind } from "./protocol";
import { TASKS, buildPrompt, type TaskSpec } from "./tasks";
import { log as defaultLog, type Logger } from "./log";

export interface RunInput {
  /** The request's id, for the log. */
  id?: string;
  task: TaskKind;
  text: string;
  context?: string;
  signal: AbortSignal;
  onDelta: (text: string) => void;
  onStatus: (message: string) => void;
}

export interface RunResult {
  text: string;
  data?: unknown;
}

/** Runs one assistant task and resolves with its reply. */
export type Runner = (input: RunInput) => Promise<RunResult>;

// Claude Code needs a working directory. The tasks don't touch files, so give
// them an empty one rather than wherever the service was started.
let scratchDir: string | undefined;
function workDir(): string {
  scratchDir ??= mkdtempSync(join(tmpdir(), "md-wingman-"));
  return scratchDir;
}

/**
 * The real runner: drives Claude through the Agent SDK, which runs the user's
 * Claude Code. `auth` decides which credential Claude Code may see (see auth.ts).
 */
export const makeAgentRunner = (auth: AuthMode, sdk: { query: QueryFn } = { query }, log: Logger = defaultLog): Runner => {
  // Built once, so a bad configuration fails at startup rather than per task.
  const env = { ...claudeEnv(auth), CLAUDE_AGENT_SDK_CLIENT_APP: "md-wingman/0.1.0" };
  return (input) => runAgent(input, env, sdk.query, log);
};

/** The Agent SDK's `query`. Tests pass a scripted one to `makeAgentRunner`. */
export type QueryFn = (args: Parameters<typeof query>[0]) => AsyncIterable<SDKMessage>;

async function runAgent(
  { id = "?", task, text, context, signal, onDelta, onStatus }: RunInput,
  env: Record<string, string | undefined>,
  query: QueryFn,
  log: Logger,
): Promise<RunResult> {
  const spec = TASKS[task];
  const claudePath = resolveClaudePath();
  log.debug(
    `task ${id}: Claude Code ${claudePath ?? "(bundled)"}, model ${process.env.WINGMAN_MODEL || "default"}, tools [${spec.tools.join(", ")}], max ${spec.maxTurns} turns`,
  );
  const abortController = new AbortController();
  const onAbort = () => abortController.abort();
  signal.addEventListener("abort", onAbort, { once: true });

  try {
    const q = query({
      prompt: buildPrompt(text, context),
      options: {
        systemPrompt: spec.systemPrompt,
        tools: spec.tools,
        allowedTools: spec.tools,
        // Anything not listed above is refused instead of prompting: there is
        // no one at a terminal to answer a permission prompt.
        permissionMode: "dontAsk",
        maxTurns: spec.maxTurns,
        outputFormat: spec.schema ? { type: "json_schema", schema: spec.schema } : undefined,
        model: process.env.WINGMAN_MODEL || undefined,
        pathToClaudeCodeExecutable: claudePath,
        includePartialMessages: true,
        persistSession: false,
        // Don't pick up the user's CLAUDE.md files or project settings.
        settingSources: [],
        cwd: workDir(),
        abortController,
        env,
        stderr: (data: string) => {
          for (const line of data.split("\n")) if (line.trim()) log.debug(`task ${id}: claude: ${line.trim()}`);
        },
      },
    });

    let streamed = "";
    for await (const msg of q) {
      if (msg.type === "stream_event" && msg.parent_tool_use_id === null) {
        const ev = msg.event;
        if (ev.type === "content_block_delta" && ev.delta.type === "text_delta") {
          streamed += ev.delta.text;
          onDelta(ev.delta.text);
        } else if (ev.type === "content_block_start" && ev.content_block.type === "server_tool_use") {
          onStatus(`Using ${ev.content_block.name}`);
        } else if (ev.type === "content_block_start" && ev.content_block.type === "tool_use") {
          // StructuredOutput is how the final answer is delivered, not a step to report.
          onStatus(ev.content_block.name === "StructuredOutput" ? "Writing up" : `Using ${ev.content_block.name}`);
        }
      } else if (msg.type === "system" && msg.subtype === "init") {
        log.info(`task ${id}: Claude Code ${msg.claude_code_version ?? "?"}, model ${msg.model}, credential ${msg.apiKeySource}`);
      } else if (msg.type === "assistant" && msg.parent_tool_use_id === null) {
        for (const block of msg.message.content) {
          if (block.type === "tool_use" || block.type === "server_tool_use") log.debug(`task ${id}: tool ${block.name}`);
        }
      } else if (msg.type === "result") {
        log.info(`task ${id}: ${resultSummary(msg)}`);
        if (msg.subtype === "success" && msg.is_error) log.warn(`task ${id}: Claude said: ${msg.result?.trim()}`);
        return taskResult(msg, spec, streamed);
      }
    }
    if (signal.aborted) throw new Error("cancelled");
    log.warn(`task ${id}: the SDK ended without a result message`);
    return { text: streamed };
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
}

/** A result message's outcome, time, turns, cost and tokens, for the log. */
export function resultSummary(msg: Partial<SDKResultMessage> & Pick<SDKResultMessage, "subtype">): string {
  const u = (msg.usage ?? {}) as Partial<Record<string, number>>;
  const tokens = [
    `in ${u.input_tokens ?? 0}`,
    `out ${u.output_tokens ?? 0}`,
    `cache read ${u.cache_read_input_tokens ?? 0}`,
    `cache write ${u.cache_creation_input_tokens ?? 0}`,
  ].join(", ");
  const cost = typeof msg.total_cost_usd === "number" ? `$${msg.total_cost_usd.toFixed(4)}` : "?";
  return `result ${msg.subtype}${msg.is_error ? " (error)" : ""} in ${msg.duration_ms ?? "?"}ms (API ${msg.duration_api_ms ?? "?"}ms), ${msg.num_turns ?? "?"} turns, ${cost}, tokens ${tokens}`;
}

/** What to tell the writer when Claude Code isn't signed in. */
export const NOT_SIGNED_IN = `Claude isn't signed in. Open Terminal, run "claude auth login", then try again.`;

/**
 * Rewrites Claude Code's errors that only make sense inside Claude Code, such
 * as "Not logged in · Please run /login", into steps the writer can follow.
 */
export function friendlyError(message: string): string {
  if (/not logged in|please run \/login/i.test(message)) return NOT_SIGNED_IN;
  return message;
}

/**
 * Turns the SDK's result message into the task's reply, or throws with a
 * message the writer can act on.
 *
 * A turn that ends on an API error (for example "Not logged in · Please run
 * /login") still has subtype "success", but with `is_error` set and the error
 * text in `result`, so check that before looking for the structured reply.
 */
export function taskResult(
  msg: Pick<SDKResultMessage, "subtype" | "is_error"> & { result?: string; structured_output?: unknown },
  spec: Pick<TaskSpec, "schema">,
  streamed = "",
): RunResult {
  if (msg.subtype !== "success") throw new Error(`Claude stopped early (${msg.subtype})`);
  if (msg.is_error) throw new Error(friendlyError(msg.result?.trim() || "Claude returned an error"));
  if (spec.schema) {
    if (msg.structured_output === undefined) throw new Error("Claude didn't return a structured reply");
    const data = msg.structured_output as { summary?: unknown };
    return { text: typeof data.summary === "string" ? data.summary : "", data };
  }
  return { text: msg.result || streamed };
}

/**
 * A stand-in runner for tests and UI work without a Claude login. It echoes a
 * canned reply word by word. Enabled with WINGMAN_FAKE=1.
 */
export const fakeRunner: Runner = async ({ task, text, signal, onDelta, onStatus }) => {
  if (task !== "copy_edit") onStatus("Using WebSearch");
  const reply = `[fake ${task}] ${text}`;
  for (const word of reply.split(/(?<= )/)) {
    if (signal.aborted) throw new Error("cancelled");
    onDelta(word);
    await Bun.sleep(5);
  }
  return { text: reply, data: fakeData(task, text) };
};

/** Canned structured replies, so the app's suggestion UI can be exercised offline. */
function fakeData(task: TaskKind, text: string): unknown {
  if (task === "copy_edit") {
    const fixes: Array<[string, string]> = [
      ["teh", "the"],
      ["Their are", "There are"],
      ["it were", "it was"],
    ];
    const edits = fixes
      .filter(([original]) => text.includes(original))
      .map(([original, replacement]) => ({ original, replacement, reason: "[fake] example fix" }));
    return { summary: `[fake] ${edits.length} suggested edits`, edits };
  }
  if (task === "fact_check") {
    const first = text.split(/(?<=[.!?])\s/)[0]?.trim() ?? text;
    return {
      summary: "[fake] one claim checked",
      claims: [
        {
          quote: first,
          verdict: "unverifiable",
          explanation: "[fake] This is a canned reply from WINGMAN_FAKE=1.",
          sources: [{ title: "Example source", url: "https://example.com" }],
        },
      ],
    };
  }
  if (task === "find_references") {
    const first = text.split(/(?<=[.!?])\s/)[0]?.trim() ?? text;
    return {
      summary: "[fake] two example references",
      references: [
        {
          quote: first,
          title: "An Example Study",
          authors: "Doe, J. and Roe, R.",
          year: "2020",
          publisher: "Journal of Examples",
          url: "https://example.com/study",
          relation: "supports",
          note: "[fake] A canned reference from WINGMAN_FAKE=1.",
        },
        {
          quote: first,
          title: "Example Encyclopedia Entry",
          authors: "",
          year: "",
          publisher: "example.org",
          url: "https://example.org/entry",
          relation: "background",
          note: "[fake] Background reading.",
        },
      ],
    };
  }
  return undefined;
}

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { query } from "@anthropic-ai/claude-agent-sdk";
import { resolveClaudePath } from "./claudePath";
import type { TaskKind } from "./protocol";
import { TASKS, buildPrompt } from "./tasks";

export interface RunInput {
  task: TaskKind;
  text: string;
  context?: string;
  signal: AbortSignal;
  onDelta: (text: string) => void;
  onStatus: (message: string) => void;
}

/** Runs one assistant task and resolves with the full reply text. */
export type Runner = (input: RunInput) => Promise<string>;

// Claude Code needs a working directory. The tasks don't touch files, so give
// them an empty one rather than wherever the service was started.
let scratchDir: string | undefined;
function workDir(): string {
  scratchDir ??= mkdtempSync(join(tmpdir(), "md-wingman-"));
  return scratchDir;
}

/**
 * The real runner: drives Claude through the Agent SDK. Authentication is
 * whatever Claude Code itself uses on this machine, so a `claude` login with a
 * Claude subscription works, as does ANTHROPIC_API_KEY.
 */
export const agentRunner: Runner = async ({ task, text, context, signal, onDelta, onStatus }) => {
  const spec = TASKS[task];
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
        model: process.env.WINGMAN_MODEL || undefined,
        pathToClaudeCodeExecutable: resolveClaudePath(),
        includePartialMessages: true,
        persistSession: false,
        // Don't pick up the user's CLAUDE.md files or project settings.
        settingSources: [],
        cwd: workDir(),
        abortController,
        env: { ...process.env, CLAUDE_AGENT_SDK_CLIENT_APP: "md-wingman/0.1.0" },
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
          onStatus(`Using ${ev.content_block.name}`);
        }
      } else if (msg.type === "result") {
        if (msg.subtype === "success") return msg.result || streamed;
        throw new Error(`Claude stopped early (${msg.subtype})`);
      }
    }
    if (signal.aborted) throw new Error("cancelled");
    return streamed;
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
};

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
  return reply;
};

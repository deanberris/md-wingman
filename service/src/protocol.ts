// Wire protocol between the md-wingman app and this service.
//
// Transport: a Unix domain socket. Each message is one JSON object on its own
// line (newline-delimited JSON). The app sends requests; the service answers
// with a stream of events that carry the request's `id`. Several requests can
// be in flight on one connection at once.

export const PROTOCOL_VERSION = 1;

export type TaskKind = "copy_edit" | "fact_check" | "find_references";

export const TASK_KINDS: readonly TaskKind[] = ["copy_edit", "fact_check", "find_references"];

/** App -> service */
export type Request =
  | { id: string; type: "hello" }
  | {
      id: string;
      type: "task";
      task: TaskKind;
      /** The text the writer selected, or the paragraph under the cursor. */
      text: string;
      /** Optional surrounding document text, for context only. */
      context?: string;
    }
  | { id: string; type: "cancel"; target: string }
  | { id: string; type: "shutdown" };

/** Service -> app */
export type Event =
  | {
      id: string;
      type: "hello";
      protocol: number;
      version: string;
      /**
       * Which credential the service uses: the Claude plan, an API key, or
       * another model provider.
       */
      auth: "subscription" | "api_key" | "provider";
      /** In "provider" mode: the provider's name and the model in use. */
      provider?: string;
      model?: string;
    }
  | { id: string; type: "delta"; text: string }
  | { id: string; type: "status"; message: string }
  | {
      id: string;
      type: "done";
      /** The reply as Markdown, or a one-line summary when `data` is set. */
      text: string;
      /** The structured reply, for tasks that have a schema (see tasks.ts). */
      data?: unknown;
    }
  | { id: string; type: "error"; message: string };

export function parseRequest(line: string): Request | { error: string; id?: string } {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return { error: "invalid JSON" };
  }
  if (typeof value !== "object" || value === null) return { error: "request must be an object" };
  const v = value as Record<string, unknown>;
  const id = typeof v.id === "string" ? v.id : undefined;
  if (!id) return { error: "missing id" };
  switch (v.type) {
    case "hello":
    case "shutdown":
      return { id, type: v.type };
    case "cancel":
      if (typeof v.target !== "string") return { id, error: "cancel needs a target id" };
      return { id, type: "cancel", target: v.target };
    case "task":
      return parseTask(id, v);
    default:
      return { id, error: `unknown request type: ${String(v.type)}` };
  }
}

function parseTask(id: string, v: Record<string, unknown>): Request | { error: string; id: string } {
  if (!TASK_KINDS.includes(v.task as TaskKind)) return { id, error: `unknown task: ${String(v.task)}` };
  if (typeof v.text !== "string" || v.text.trim() === "") return { id, error: "task needs non-empty text" };
  if (v.context !== undefined && typeof v.context !== "string") return { id, error: "context must be a string" };
  return { id, type: "task", task: v.task as TaskKind, text: v.text, context: v.context as string | undefined };
}

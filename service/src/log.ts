// The service's log. Lines go to stderr as "LEVEL message"; the app reads
// them and writes them to its log file under the "service" target, so the
// level word must come first.
//
// WINGMAN_LOG sets the level: error, warn, info (default), debug or trace.
// Secrets are scrubbed: API keys (sk-…), bearer tokens, and the values of
// credential variables from the environment. Document text is never logged
// unless WINGMAN_LOG_TEXT=1; use `textInfo` to describe it instead.

export type Level = "error" | "warn" | "info" | "debug" | "trace";

const ORDER: Record<Level, number> = { error: 0, warn: 1, info: 2, debug: 3, trace: 4 };

/** Environment variables whose values must never be logged. */
const SECRET_VARS = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_FOUNDRY_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "WINGMAN_JEV_KEY"];

export function parseLevel(value: string | undefined): Level {
  const v = (value ?? "").trim().toLowerCase();
  if (v === "warning") return "warn";
  return v in ORDER ? (v as Level) : "info";
}

/** Replaces known secrets, API keys and bearer tokens with [redacted]. */
export function scrub(line: string, secrets: string[] = []): string {
  let out = line;
  for (const s of secrets) if (s.length >= 8) out = out.split(s).join("[redacted]");
  return out.replace(/sk-[A-Za-z0-9_\-.=]{16,}/g, "sk-[redacted]").replace(/Bearer [A-Za-z0-9_\-.=]{8,}/g, "Bearer [redacted]");
}

/** A short, stable fingerprint (FNV-1a), matching the app's. */
export function fingerprint(text: string): string {
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(text)) h = Math.imul(h ^ b, 0x01000193) >>> 0;
  return h.toString(16).padStart(8, "0");
}

export interface Logger {
  level: Level;
  includeText: boolean;
  error(msg: string): void;
  warn(msg: string): void;
  info(msg: string): void;
  debug(msg: string): void;
  trace(msg: string): void;
  enabled(level: Level): boolean;
  /** Document text as its size and fingerprint, and the text only with WINGMAN_LOG_TEXT=1. */
  text(text: string | undefined): string;
}

export function createLogger(
  env: NodeJS.ProcessEnv = process.env,
  write: (line: string) => void = (line) => process.stderr.write(line + "\n"),
): Logger {
  const level = parseLevel(env.WINGMAN_LOG);
  const includeText = env.WINGMAN_LOG_TEXT === "1";
  const secrets = SECRET_VARS.map((name) => env[name]).filter((v): v is string => !!v);
  const enabled = (l: Level) => ORDER[l] <= ORDER[level];
  const emit = (l: Level) => (msg: string) => {
    if (enabled(l)) write(scrub(`${l.toUpperCase()} ${msg}`, secrets).replace(/\n/g, "\n    "));
  };
  return {
    level,
    includeText,
    enabled,
    error: emit("error"),
    warn: emit("warn"),
    info: emit("info"),
    debug: emit("debug"),
    trace: emit("trace"),
    text: (text) => {
      if (text === undefined) return "[none]";
      const words = text.split(/\s+/).filter(Boolean).length;
      const summary = `[${[...text].length} chars, ${words} words, #${fingerprint(text)}]`;
      return includeText ? `${summary} ${JSON.stringify(text.slice(0, 2000))}` : summary;
    },
  };
}

/** The service's logger, set up from the environment. Quiet under `bun test`. */
export const log: Logger = createLogger(
  process.env.NODE_ENV === "test" && !process.env.WINGMAN_LOG ? { ...process.env, WINGMAN_LOG: "error" } : process.env,
);

/** A logger that drops everything, for tests. */
export const silent: Logger = createLogger({ WINGMAN_LOG: "error" }, () => {});

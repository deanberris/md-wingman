import { expect, test } from "bun:test";
import { createLogger, fingerprint, oneLine, parseLevel, scrub } from "./log";

test("levels filter lines and lead each one", () => {
  const lines: string[] = [];
  const log = createLogger({ WINGMAN_LOG: "warn" }, (l) => lines.push(l));
  log.info("quiet");
  log.warn("loud");
  log.error("louder");
  expect(lines).toEqual(["WARN loud", "ERROR louder"]);
  expect(parseLevel("DEBUG")).toBe("debug");
  expect(parseLevel("nonsense")).toBe("info");
});

test("secrets from the environment, API keys and tokens are scrubbed", () => {
  const lines: string[] = [];
  const log = createLogger({ CLAUDE_CODE_OAUTH_TOKEN: "oauth-token-value-123" }, (l) => lines.push(l));
  log.info("token oauth-token-value-123 and sk-ant-api03-abcdefghijklmnopqrstuvwxyz and Bearer abc.def-ghi_jkl");
  expect(lines[0]).toBe("INFO token [redacted] and sk-[redacted] and Bearer [redacted]");
  expect(scrub("sk-short stays")).toBe("sk-short stays");
});

test("document text is described, not logged, by default", () => {
  const hidden = createLogger({}, () => {}).text("Their going home.");
  expect(hidden).toBe(`[17 chars, 3 words, #${fingerprint("Their going home.")}]`);
  const shown = createLogger({ WINGMAN_LOG_TEXT: "1" }, () => {}).text("Their going home.");
  expect(shown).toEndWith(` "Their going home."`);
});

test("a stack trace stays on one line, and crashes are FATAL", () => {
  const lines: string[] = [];
  const log = createLogger({}, (l) => lines.push(l));
  log.fatal("uncaught: Error: boom\n    at main (main.ts:1:1)");
  expect(lines).toEqual(["FATAL uncaught: Error: boom\\n    at main (main.ts:1:1)"]);
  expect(oneLine("C:\\path\nnext")).toBe("C:\\\\path\\nnext");
  // FATAL always shows, even at the quietest level.
  const quiet: string[] = [];
  createLogger({ WINGMAN_LOG: "error" }, (l) => quiet.push(l)).fatal("x");
  expect(quiet).toEqual(["FATAL x"]);
});

test("fingerprints match the app's FNV-1a", () => {
  // The same value as wingman_log::fingerprint("abc").
  expect(fingerprint("abc")).toBe("1a47e90b");
});

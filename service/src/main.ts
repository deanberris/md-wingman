// Entry point for the md-wingman assistant service.
//
// Usage: wingman-service --socket <path>
//
// The app starts this process and stops it when it quits. Logs go to stderr
// as "LEVEL message" lines, which the app writes to its log file (see log.ts).
// Set WINGMAN_FAKE=1 to answer with canned replies instead of calling Claude.
// WINGMAN_AUTH picks the sign-in mode: subscription (default), api_key, or
// provider for another Anthropic-compatible endpoint (see auth.ts).

import { authModeFromEnv, providerFromEnv, type AuthMode, type Provider } from "./auth";
import { fakeRunner, makeAgentRunner, type Runner } from "./runner";
import { SERVICE_VERSION, startServer } from "./server";
import { log } from "./log";
import { resolveClaudePath } from "./claudePath";

function argValue(name: string): string | undefined {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const socketPath = argValue("--socket");
if (!socketPath) {
  console.error("usage: wingman-service --socket <path>");
  process.exit(2);
}

log.info(
  `wingman-service ${SERVICE_VERSION} starting: pid ${process.pid}, bun ${Bun.version}, ${process.platform}/${process.arch}, log level ${log.level}${log.includeText ? ", logging document text" : ""}`,
);
const fake = process.env.WINGMAN_FAKE === "1";
if (fake) log.warn("WINGMAN_FAKE=1: using canned replies, not Claude");
if (process.env.WINGMAN_MODEL) log.info(`model: ${process.env.WINGMAN_MODEL}`);

let auth: AuthMode;
let provider: Provider | undefined;
let runner: Runner;
try {
  auth = authModeFromEnv();
  if (auth === "provider") provider = providerFromEnv();
  runner = fake ? fakeRunner : makeAgentRunner(auth, undefined, log, provider);
} catch (err) {
  log.error(err instanceof Error ? err.message : String(err));
  process.exit(2);
}
log.info(`sign-in mode: ${auth}`);
if (provider) {
  log.info(
    `model provider: ${provider.name} at ${provider.baseUrl}, model ${provider.model}, fast model ${provider.fastModel ?? provider.model}, ${provider.token ? "with" : "without"} a token, web search ${provider.webSearch ? "on" : "off"}`,
  );
}
if (!fake) {
  try {
    log.info(`Claude Code: ${resolveClaudePath() ?? "the Agent SDK's bundled copy"}`);
  } catch (err) {
    // Not fatal here: each task reports it to the writer.
    log.error(err instanceof Error ? err.message : String(err));
  }
}

const server = startServer({
  socketPath,
  runner,
  auth,
  provider: provider && { name: provider.name, model: provider.model },
  log,
  onShutdown: () => shutdown("the app asked"),
});

function shutdown(reason = "asked to") {
  log.info(`shutting down (${reason})`);
  server.stop();
  process.exit(0);
}

process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));
process.on("uncaughtException", (err) => {
  log.fatal(`wingman-service crashed: ${err.stack ?? err}`);
  process.exit(1);
});
process.on("unhandledRejection", (err) => log.error(`unhandled rejection: ${err instanceof Error ? (err.stack ?? err.message) : String(err)}`));

// If the app dies without stopping us, our stdin closes: exit too, so no
// orphaned service is left running.
process.stdin.on("end", () => shutdown("the app went away"));
process.stdin.resume();

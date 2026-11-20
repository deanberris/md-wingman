// Entry point for the md-wingman assistant service.
//
// Usage: wingman-service --socket <path>
//
// The app starts this process and stops it when it quits. Logs go to stderr.
// Set WINGMAN_FAKE=1 to answer with canned replies instead of calling Claude.
// WINGMAN_AUTH picks the sign-in mode: subscription (default) or api_key.

import { authModeFromEnv, type AuthMode } from "./auth";
import { fakeRunner, makeAgentRunner, type Runner } from "./runner";
import { startServer } from "./server";

function argValue(name: string): string | undefined {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const socketPath = argValue("--socket");
if (!socketPath) {
  console.error("usage: wingman-service --socket <path>");
  process.exit(2);
}

const log = (msg: string) => console.error(`[wingman-service] ${msg}`);
const fake = process.env.WINGMAN_FAKE === "1";
if (fake) log("WINGMAN_FAKE=1: using canned replies, not Claude");

let auth: AuthMode;
let runner: Runner;
try {
  auth = authModeFromEnv();
  runner = fake ? fakeRunner : makeAgentRunner(auth);
} catch (err) {
  log(err instanceof Error ? err.message : String(err));
  process.exit(2);
}
log(`sign-in mode: ${auth}`);

const server = startServer({
  socketPath,
  runner,
  auth,
  log,
  onShutdown: () => shutdown(),
});

function shutdown() {
  server.stop();
  process.exit(0);
}

process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);

// If the app dies without stopping us, our stdin closes: exit too, so no
// orphaned service is left running.
process.stdin.on("end", shutdown);
process.stdin.resume();

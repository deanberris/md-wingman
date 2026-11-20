// Entry point for the md-wingman assistant service.
//
// Usage: wingman-service --socket <path>
//
// The app starts this process and stops it when it quits. Logs go to stderr.
// Set WINGMAN_FAKE=1 to answer with canned replies instead of calling Claude.

import { agentRunner, fakeRunner } from "./runner";
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

const server = startServer({
  socketPath,
  runner: fake ? fakeRunner : agentRunner,
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

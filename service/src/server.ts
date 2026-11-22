import { chmodSync, existsSync, unlinkSync } from "node:fs";
import type { Socket } from "bun";
import type { AuthMode } from "./auth";
import { type Logger, silent } from "./log";
import { type Event, PROTOCOL_VERSION, parseRequest, type Request } from "./protocol";
import type { Runner } from "./runner";

export const SERVICE_VERSION = "0.1.0";

interface ConnState {
  conn: number;
  buffer: string;
  inflight: Map<string, AbortController>;
}

export interface ServerOptions {
  socketPath: string;
  runner: Runner;
  /** Reported to clients in the hello reply. */
  auth?: AuthMode;
  /** Reported in the hello reply in "provider" mode. */
  provider?: { name: string; model: string };
  log?: Logger;
  /** Called when a client asks the service to shut down. */
  onShutdown?: () => void;
}

export function startServer({
  socketPath,
  runner,
  auth = "subscription",
  provider,
  log = silent,
  onShutdown,
}: ServerOptions) {
  let connections = 0;
  // A stale socket file from a crashed run would make listen() fail.
  if (existsSync(socketPath)) unlinkSync(socketPath);

  const send = (sock: Socket<ConnState>, ev: Event) => {
    sock.write(`${JSON.stringify(ev)}\n`);
  };

  const handle = (sock: Socket<ConnState>, req: Request) => {
    switch (req.type) {
      case "hello":
        log.debug(`conn ${sock.data.conn}: hello`);
        send(sock, {
          id: req.id,
          type: "hello",
          protocol: PROTOCOL_VERSION,
          version: SERVICE_VERSION,
          auth,
          ...(provider && { provider: provider.name, model: provider.model }),
        });
        return;
      case "cancel": {
        const ac = sock.data.inflight.get(req.target);
        log.info(`task ${req.target} cancel requested${ac ? "" : " (not running)"}`);
        ac?.abort();
        return;
      }
      case "shutdown":
        log.info("shutdown requested");
        onShutdown?.();
        return;
      case "task": {
        if (sock.data.inflight.has(req.id)) {
          log.warn(`task ${req.id} refused: a request with this id is already running`);
          send(sock, { id: req.id, type: "error", message: "a request with this id is already running" });
          return;
        }
        const ac = new AbortController();
        sock.data.inflight.set(req.id, ac);
        const started = performance.now();
        const elapsed = () => `${Math.round(performance.now() - started)}ms`;
        log.info(
          `task ${req.id} ${req.task} started: text ${log.text(req.text)}, context ${log.text(req.context)} (${sock.data.inflight.size} running)`,
        );
        runner({
          id: req.id,
          task: req.task,
          text: req.text,
          context: req.context,
          signal: ac.signal,
          onDelta: (text) => send(sock, { id: req.id, type: "delta", text }),
          onStatus: (message) => {
            log.debug(`task ${req.id} status: ${message}`);
            send(sock, { id: req.id, type: "status", message });
          },
        })
          .then(({ text, data }) => {
            log.info(`task ${req.id} ${req.task} done in ${elapsed()}: ${summarize(data)}`);
            send(sock, { id: req.id, type: "done", text, data });
          })
          .catch((err: unknown) => {
            const message = ac.signal.aborted ? "cancelled" : errorMessage(err);
            logFailure(log, `task ${req.id} ${req.task}`, message, elapsed(), err);
            send(sock, { id: req.id, type: "error", message });
          })
          .finally(() => sock.data.inflight.delete(req.id));
        return;
      }
    }
  };

  const server = Bun.listen<ConnState>({
    unix: socketPath,
    socket: {
      open(sock) {
        sock.data = { conn: ++connections, buffer: "", inflight: new Map() };
        log.info(`conn ${sock.data.conn}: app connected`);
      },
      data(sock, chunk) {
        sock.data.buffer += chunk.toString();
        for (let nl = sock.data.buffer.indexOf("\n"); nl >= 0; nl = sock.data.buffer.indexOf("\n")) {
          const line = sock.data.buffer.slice(0, nl).trim();
          sock.data.buffer = sock.data.buffer.slice(nl + 1);
          if (line === "") continue;
          const parsed = parseRequest(line);
          if ("error" in parsed) {
            log.warn(`conn ${sock.data.conn}: bad request (${parsed.error}), ${line.length} bytes`);
            send(sock, { id: parsed.id ?? "", type: "error", message: parsed.error });
          } else {
            handle(sock, parsed);
          }
        }
      },
      close(sock) {
        // The app went away: stop any work it was waiting for.
        const n = sock.data.inflight.size;
        log.info(`conn ${sock.data.conn}: app disconnected${n ? `, cancelling ${n} running tasks` : ""}`);
        for (const ac of sock.data.inflight.values()) ac.abort();
      },
    },
  });

  // Only the user who started the service may connect.
  chmodSync(socketPath, 0o600);
  log.info(`listening on ${socketPath}`);

  return {
    stop() {
      log.info("stopping");
      server.stop(true);
      if (existsSync(socketPath)) unlinkSync(socketPath);
    },
  };
}

/** What a structured reply holds, by count, without its text. */
export function summarize(data: unknown): string {
  if (typeof data !== "object" || data === null) return "no structured data";
  const parts = Object.entries(data as Record<string, unknown>)
    .filter(([, v]) => Array.isArray(v))
    .map(([k, v]) => `${(v as unknown[]).length} ${k}`);
  return parts.length ? parts.join(", ") : "structured data";
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Logs how a task ended when it didn't succeed. */
function logFailure(log: Logger, what: string, message: string, elapsed: string, err: unknown) {
  if (message === "cancelled") {
    log.info(`${what} cancelled after ${elapsed}`);
    return;
  }
  log.error(`${what} failed after ${elapsed}: ${message}`);
  if (err instanceof Error && err.stack) log.debug(err.stack);
}

import { chmodSync, existsSync, unlinkSync } from "node:fs";
import type { Socket } from "bun";
import { PROTOCOL_VERSION, parseRequest, type Event, type Request } from "./protocol";
import type { AuthMode } from "./auth";
import type { Runner } from "./runner";

export const SERVICE_VERSION = "0.1.0";

interface ConnState {
  buffer: string;
  inflight: Map<string, AbortController>;
}

export interface ServerOptions {
  socketPath: string;
  runner: Runner;
  /** Reported to clients in the hello reply. */
  auth?: AuthMode;
  log?: (msg: string) => void;
  /** Called when a client asks the service to shut down. */
  onShutdown?: () => void;
}

export function startServer({ socketPath, runner, auth = "subscription", log = () => {}, onShutdown }: ServerOptions) {
  // A stale socket file from a crashed run would make listen() fail.
  if (existsSync(socketPath)) unlinkSync(socketPath);

  const send = (sock: Socket<ConnState>, ev: Event) => {
    sock.write(JSON.stringify(ev) + "\n");
  };

  const handle = (sock: Socket<ConnState>, req: Request) => {
    switch (req.type) {
      case "hello":
        send(sock, { id: req.id, type: "hello", protocol: PROTOCOL_VERSION, version: SERVICE_VERSION, auth });
        return;
      case "cancel":
        sock.data.inflight.get(req.target)?.abort();
        return;
      case "shutdown":
        log("shutdown requested");
        onShutdown?.();
        return;
      case "task": {
        if (sock.data.inflight.has(req.id)) {
          send(sock, { id: req.id, type: "error", message: "a request with this id is already running" });
          return;
        }
        const ac = new AbortController();
        sock.data.inflight.set(req.id, ac);
        log(`task ${req.task} ${req.id}`);
        runner({
          task: req.task,
          text: req.text,
          context: req.context,
          signal: ac.signal,
          onDelta: (text) => send(sock, { id: req.id, type: "delta", text }),
          onStatus: (message) => send(sock, { id: req.id, type: "status", message }),
        })
          .then((text) => send(sock, { id: req.id, type: "done", text }))
          .catch((err: unknown) => {
            const message = ac.signal.aborted ? "cancelled" : err instanceof Error ? err.message : String(err);
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
        sock.data = { buffer: "", inflight: new Map() };
      },
      data(sock, chunk) {
        sock.data.buffer += chunk.toString();
        let nl: number;
        while ((nl = sock.data.buffer.indexOf("\n")) >= 0) {
          const line = sock.data.buffer.slice(0, nl).trim();
          sock.data.buffer = sock.data.buffer.slice(nl + 1);
          if (line === "") continue;
          const parsed = parseRequest(line);
          if ("error" in parsed) {
            send(sock, { id: parsed.id ?? "", type: "error", message: parsed.error });
          } else {
            handle(sock, parsed);
          }
        }
      },
      close(sock) {
        // The app went away: stop any work it was waiting for.
        for (const ac of sock.data.inflight.values()) ac.abort();
      },
    },
  });

  // Only the user who started the service may connect.
  chmodSync(socketPath, 0o600);
  log(`listening on ${socketPath}`);

  return {
    stop() {
      server.stop(true);
      if (existsSync(socketPath)) unlinkSync(socketPath);
    },
  };
}

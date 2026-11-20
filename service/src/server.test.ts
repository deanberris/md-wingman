import { afterEach, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Event } from "./protocol";
import { fakeRunner, type Runner } from "./runner";
import { startServer } from "./server";

let stop: (() => void) | undefined;
afterEach(() => stop?.());

async function connect(runner: Runner) {
  const socketPath = join(tmpdir(), `wingman-test-${process.pid}-${Math.random().toString(36).slice(2)}.sock`);
  stop = startServer({ socketPath, runner }).stop;
  const events: Event[] = [];
  let buf = "";
  const waiters: Array<() => void> = [];
  const sock = await Bun.connect({
    unix: socketPath,
    socket: {
      data(_s, chunk) {
        buf += chunk.toString();
        let nl: number;
        while ((nl = buf.indexOf("\n")) >= 0) {
          events.push(JSON.parse(buf.slice(0, nl)));
          buf = buf.slice(nl + 1);
        }
        waiters.splice(0).forEach((w) => w());
      },
    },
  });
  const send = (obj: unknown) => sock.write(JSON.stringify(obj) + "\n");
  const waitFor = async (pred: (e: Event) => boolean) => {
    while (!events.some(pred)) await new Promise<void>((r) => waiters.push(r));
    return events.find(pred)!;
  };
  return { send, waitFor, events, close: () => sock.end() };
}

test("hello reports the protocol version", async () => {
  const c = await connect(fakeRunner);
  c.send({ id: "h", type: "hello" });
  const ev = await c.waitFor((e) => e.id === "h");
  expect(ev).toMatchObject({ type: "hello", protocol: 1, auth: "subscription" });
  c.close();
});

test("a task streams deltas then done", async () => {
  const c = await connect(fakeRunner);
  c.send({ id: "t1", type: "task", task: "copy_edit", text: "teh cat sat" });
  const done = await c.waitFor((e) => e.id === "t1" && e.type === "done");
  const deltas = c.events.filter((e) => e.id === "t1" && e.type === "delta").map((e) => (e as { text: string }).text);
  expect(deltas.length).toBeGreaterThan(1);
  expect(deltas.join("")).toBe((done as { text: string }).text);
  expect((done as { data: { edits: unknown[] } }).data.edits).toEqual([
    { original: "teh", replacement: "the", reason: "[fake] example fix" },
  ]);
  c.close();
});

test("find_references returns structured references", async () => {
  const c = await connect(fakeRunner);
  c.send({ id: "r1", type: "task", task: "find_references", text: "Coffee helps. Tea too." });
  const done = await c.waitFor((e) => e.id === "r1" && e.type === "done");
  const refs = (done as { data: { references: Array<{ quote: string; relation: string }> } }).data.references;
  expect(refs.length).toBe(2);
  expect(refs[0]).toMatchObject({ quote: "Coffee helps.", relation: "supports" });
  c.close();
});

test("bad requests get an error event", async () => {
  const c = await connect(fakeRunner);
  c.send({ id: "b", type: "task", task: "summarise", text: "x" });
  const ev = await c.waitFor((e) => e.id === "b");
  expect(ev).toMatchObject({ type: "error" });
  c.close();
});

test("cancel stops a running task", async () => {
  const slow: Runner = async ({ signal }) => {
    await new Promise((_, reject) => signal.addEventListener("abort", () => reject(new Error("aborted"))));
    return { text: "never" };
  };
  const c = await connect(slow);
  c.send({ id: "s", type: "task", task: "fact_check", text: "The moon is cheese." });
  await Bun.sleep(20);
  c.send({ id: "c", type: "cancel", target: "s" });
  const ev = await c.waitFor((e) => e.id === "s");
  expect(ev).toMatchObject({ type: "error", message: "cancelled" });
  c.close();
});

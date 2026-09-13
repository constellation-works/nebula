// The window's side of the layout worker: one promise per request, answers
// matched by sequence number. Where there is no `Worker` (vitest's jsdom),
// elk runs in this thread instead, so the tests exercise the real layout.

import type { ElkNode } from "elkjs/lib/elk-api";

export interface LayoutRequest {
  seq: number;
  graph: ElkNode;
}

export type LayoutReply =
  | { seq: number; ok: true; graph: ElkNode }
  | { seq: number; ok: false; error: string };

interface Pending {
  graph: ElkNode;
  resolve: (graph: ElkNode) => void;
  reject: (error: Error) => void;
}

let worker: Worker | null | undefined;
let seq = 0;
const pending = new Map<number, Pending>();

function settle(reply: LayoutReply) {
  const p = pending.get(reply.seq);
  if (p === undefined) return;
  pending.delete(reply.seq);
  if (reply.ok) p.resolve(reply.graph);
  else p.reject(new Error(reply.error));
}

/** The worker, started on first use; null where workers do not exist. */
function getWorker(): Worker | null {
  if (worker !== undefined) return worker;
  if (typeof Worker === "undefined") {
    worker = null;
    return null;
  }
  try {
    const w = new Worker(new URL("./layout.worker.ts", import.meta.url), { type: "module" });
    w.addEventListener("message", (e: MessageEvent<LayoutReply>) => settle(e.data));
    w.addEventListener("error", () => {
      // The worker could not load or crashed: finish what it owed in this
      // thread and stay there. Slower, but the graph still draws.
      w.terminate();
      worker = null;
      const owed = [...pending.values()];
      pending.clear();
      for (const p of owed) inThread(p.graph).then(p.resolve, p.reject);
    });
    worker = w;
  } catch {
    worker = null;
  }
  return worker;
}

/** The same elk, loaded on demand so the main bundle does not carry it. */
async function inThread(graph: ElkNode): Promise<ElkNode> {
  const { default: ELK } = await import("elkjs/lib/elk.bundled.js");
  return new ELK().layout(graph);
}

/** Lay the graph out, off the main thread when there is one to go to. */
export function layoutGraph(graph: ElkNode): Promise<ElkNode> {
  const w = getWorker();
  if (w === null) return inThread(graph);
  seq += 1;
  const request: LayoutRequest = { seq, graph };
  return new Promise<ElkNode>((resolve, reject) => {
    pending.set(request.seq, { graph, resolve, reject });
    w.postMessage(request);
  });
}

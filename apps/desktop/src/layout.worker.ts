// The layout thread. elk's layered algorithm on a few hundred nodes takes
// long enough to drop frames, so it runs here and the window keeps panning.
// One message in (an elk graph), one message out (the same graph, placed).

import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import type { LayoutReply, LayoutRequest } from "./layoutClient";

const elk = new ELK();

addEventListener("message", (event: MessageEvent<LayoutRequest>) => {
  const { seq, graph } = event.data;
  elk
    .layout(graph)
    .then((laid: ElkNode) => postMessage({ seq, ok: true, graph: laid } satisfies LayoutReply))
    .catch((e: unknown) => postMessage({ seq, ok: false, error: String(e) } satisfies LayoutReply));
});

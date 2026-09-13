// The pure half of the graph view: `GraphExport` in, an elk graph out, elk's
// answer back into something the SVG can draw. Nothing here touches the DOM
// or the worker, which is what makes it testable and what keeps the drawing
// honest about which edges shaped the layout.

import type { ElkExtendedEdge, ElkNode } from "elkjs/lib/elk-api";
import type { EdgeRecord } from "./types/EdgeRecord";
import type { EdgeType } from "./types/EdgeType";
import type { GraphExport } from "./types/GraphExport";
import type { NodeSummary } from "./types/NodeSummary";
import type { TagCount } from "./types/TagCount";

/** Card size, fixed so layout does not need the DOM to measure text. */
export const NODE_WIDTH = 220;
export const NODE_HEIGHT = 58;
/** Titles longer than this are cut on the card; the full title is on hover. */
export const TITLE_CHARS = 40;
/** Tag chips drawn on a card before "+n" takes over. */
export const MAX_CHIPS = 3;

/** What the toolbar narrows the drawing to. Both empty means everything. */
export interface Filter {
  /** Case-insensitive title substring. */
  query: string;
  /** A node must carry every one of these, as `neb list --tag` does. */
  tags: string[];
}

export interface Point {
  x: number;
  y: number;
}

/** A node with the place elk gave it. */
export interface PlacedNode extends NodeSummary {
  x: number;
  y: number;
  width: number;
  height: number;
  /** Declares a `reopens` edge: the card gets a marker. */
  reopens: boolean;
}

/** An edge with the line to draw it along, child first. */
export interface DrawnEdge extends EdgeRecord {
  /** Genealogy: elk's route, from the child up to the parent, so an arrowhead at the end points at the parent. Contradicts: centre to centre. */
  points: Point[];
}

export interface Layout {
  width: number;
  height: number;
  nodes: PlacedNode[];
  edges: DrawnEdge[];
}

/** Genealogy edges layer the drawing; `contradicts` is a relation, not a lineage. */
export const isGenealogy = (e: { type: EdgeType }): boolean => e.type !== "contradicts";

/** Every tag with the number of nodes carrying it, most common first. */
export function tagCounts(nodes: readonly NodeSummary[]): TagCount[] {
  const counts = new Map<string, number>();
  for (const n of nodes) {
    for (const t of n.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
  }
  return [...counts]
    .map(([tag, count]) => ({ tag, count }))
    .sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag));
}

/**
 * The nodes that pass the filter and the edges with both ends still present.
 * Applied before layout so the drawing stays compact rather than leaving holes.
 */
export function filterGraph(graph: GraphExport, filter: Filter): GraphExport {
  const q = filter.query.trim().toLowerCase();
  const nodes = graph.nodes.filter(
    (n) =>
      (q === "" || n.title.toLowerCase().includes(q)) &&
      filter.tags.every((t) => n.tags.includes(t)),
  );
  const kept = new Set(nodes.map((n) => n.id));
  const edges = graph.edges.filter((e) => kept.has(e.from) && kept.has(e.to));
  return { nodes, edges };
}

/** The elk id for an edge; both ends and the type, so parallel edges stay distinct. */
export const edgeId = (e: EdgeRecord): string => `${e.from}>${e.to}:${e.type}`;

/**
 * The layered layout's input. Only genealogy edges go in, pointed parent to
 * child so parents sit above; `contradicts` is left out and drawn afterwards.
 * Edges to nodes not in the export are dropped rather than sent to elk, which
 * would throw on the unknown id.
 */
export function toElkGraph(graph: GraphExport): ElkNode {
  const ids = new Set(graph.nodes.map((n) => n.id));
  const edges: ElkExtendedEdge[] = graph.edges
    .filter((e) => isGenealogy(e) && ids.has(e.from) && ids.has(e.to) && e.from !== e.to)
    .map((e) => ({ id: edgeId(e), sources: [e.to], targets: [e.from] }));
  return {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "DOWN",
      "elk.edgeRouting": "ORTHOGONAL",
      "elk.spacing.nodeNode": "28",
      "elk.layered.spacing.nodeNodeBetweenLayers": "56",
      "elk.spacing.edgeNode": "16",
      "elk.layered.spacing.edgeNodeBetweenLayers": "24",
      "elk.spacing.componentComponent": "48",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
      "elk.layered.cycleBreaking.strategy": "DEPTH_FIRST",
    },
    children: graph.nodes.map((n) => ({ id: n.id, width: NODE_WIDTH, height: NODE_HEIGHT })),
    edges,
  };
}

const centre = (n: PlacedNode): Point => ({ x: n.x + n.width / 2, y: n.y + n.height / 2 });

/**
 * Elk's placed graph joined back to the export: each node with its position,
 * each genealogy edge with its route reversed to run child → parent, and
 * each `contradicts` pair once, as a straight line between centres.
 */
export function fromElkLayout(laid: ElkNode, graph: GraphExport): Layout {
  const reopeners = new Set(graph.edges.filter((e) => e.type === "reopens").map((e) => e.from));
  const placed = new Map<string, PlacedNode>();
  const at = new Map((laid.children ?? []).map((c) => [c.id, c]));
  for (const n of graph.nodes) {
    const c = at.get(n.id);
    placed.set(n.id, {
      ...n,
      x: c?.x ?? 0,
      y: c?.y ?? 0,
      width: c?.width ?? NODE_WIDTH,
      height: c?.height ?? NODE_HEIGHT,
      reopens: reopeners.has(n.id),
    });
  }

  const routes = new Map<string, Point[]>();
  for (const e of laid.edges ?? []) {
    const pts: Point[] = [];
    for (const s of e.sections ?? []) {
      pts.push(s.startPoint, ...(s.bendPoints ?? []), s.endPoint);
    }
    routes.set(e.id, pts.reverse());
  }

  const edges: DrawnEdge[] = [];
  const seenPairs = new Set<string>();
  for (const e of graph.edges) {
    const from = placed.get(e.from);
    const to = placed.get(e.to);
    if (from === undefined || to === undefined) continue;
    if (isGenealogy(e)) {
      const points = routes.get(edgeId(e)) ?? [centre(from), centre(to)];
      edges.push({ ...e, points });
    } else {
      // `contradicts` is mutual, so the export carries it twice.
      const pair = [e.from, e.to].sort().join("|");
      if (seenPairs.has(pair)) continue;
      seenPairs.add(pair);
      edges.push({ ...e, points: [centre(from), centre(to)] });
    }
  }

  return {
    width: laid.width ?? 0,
    height: laid.height ?? 0,
    nodes: [...placed.values()],
    edges,
  };
}

/** Where a node came from and what came from it, along genealogy edges only. */
export interface Lineage {
  ancestors: Set<string>;
  descendants: Set<string>;
}

/** The transitive parents (`trace up`) and children (`trace down`) of one node. */
export function lineage(edges: readonly EdgeRecord[], id: string): Lineage {
  const up = new Map<string, string[]>();
  const down = new Map<string, string[]>();
  for (const e of edges) {
    if (!isGenealogy(e)) continue;
    up.set(e.from, [...(up.get(e.from) ?? []), e.to]);
    down.set(e.to, [...(down.get(e.to) ?? []), e.from]);
  }
  const walk = (next: Map<string, string[]>): Set<string> => {
    const seen = new Set<string>();
    const queue = [id];
    while (queue.length > 0) {
      const cur = queue.shift()!;
      for (const n of next.get(cur) ?? []) {
        if (n === id || seen.has(n)) continue;
        seen.add(n);
        queue.push(n);
      }
    }
    return seen;
  };
  return { ancestors: walk(up), descendants: walk(down) };
}

/** The card's title: cut with an ellipsis past `TITLE_CHARS`. */
export function truncate(title: string, max = TITLE_CHARS): string {
  return title.length <= max ? title : `${title.slice(0, max - 1).trimEnd()}…`;
}

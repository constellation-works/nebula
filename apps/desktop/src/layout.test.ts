import { describe, expect, it } from "vitest";
import {
  filterGraph,
  fromElkLayout,
  lineage,
  NODE_HEIGHT,
  NODE_WIDTH,
  tagCounts,
  toElkGraph,
  truncate,
} from "./layout";
import { layoutGraph } from "./layoutClient";
import type { GraphExport } from "./types/GraphExport";
import type { NodeSummary } from "./types/NodeSummary";

const node = (id: string, title: string, tags: string[] = [], status: NodeSummary["status"] = "seed"): NodeSummary => ({
  id,
  title,
  status,
  tags,
  created: "2026-09-01",
  updated: "2026-09-10",
});

// root ← a ← b, root ← c; b and c contradict; d reopens a refuted r.
const graph: GraphExport = {
  nodes: [
    node("root", "the first idea", ["capture"]),
    node("a", "a refinement of the first idea", ["capture", "desktop"], "hypothesis"),
    node("b", "a further refinement", ["desktop"], "hypothesis"),
    node("c", "a sideways generalisation", ["capture"]),
    node("r", "something that turned out wrong", [], "refuted"),
    node("d", "the refuted thing, revisited", ["desktop"], "hypothesis"),
  ],
  edges: [
    { from: "a", type: "derives-from", to: "root" },
    { from: "b", type: "refines", to: "a" },
    { from: "c", type: "generalizes", to: "root" },
    { from: "b", type: "contradicts", to: "c" },
    { from: "c", type: "contradicts", to: "b" },
    { from: "d", type: "reopens", to: "r" },
  ],
};

describe("toElkGraph", () => {
  it("layers on genealogy only, parent above child, and leaves contradicts out", () => {
    const elk = toElkGraph(graph);
    expect(elk.children).toHaveLength(6);
    expect(elk.children![0]).toMatchObject({ id: "root", width: NODE_WIDTH, height: NODE_HEIGHT });
    expect(elk.edges).toHaveLength(4);
    expect(elk.edges!.map((e) => e.id)).not.toContainEqual(expect.stringContaining("contradicts"));
    // The export says child → parent; elk gets parent → child so DOWN puts parents on top.
    expect(elk.edges![0]).toMatchObject({ sources: ["root"], targets: ["a"] });
    expect(elk.layoutOptions).toMatchObject({ "elk.algorithm": "layered", "elk.direction": "DOWN" });
  });

  it("drops an edge whose end is not in the export rather than handing elk an unknown id", () => {
    const dangling: GraphExport = {
      nodes: [node("x", "x")],
      edges: [{ from: "x", type: "derives-from", to: "gone" }],
    };
    expect(toElkGraph(dangling).edges).toEqual([]);
  });
});

describe("filterGraph", () => {
  it("matches titles case-insensitively and drops the edges of hidden nodes", () => {
    const out = filterGraph(graph, { query: "REFINE", tags: [] });
    expect(out.nodes.map((n) => n.id)).toEqual(["a", "b"]);
    // a→root and b↔c went with root and c; b→a stays.
    expect(out.edges).toEqual([{ from: "b", type: "refines", to: "a" }]);
  });

  it("requires every selected tag", () => {
    expect(filterGraph(graph, { query: "", tags: ["desktop"] }).nodes.map((n) => n.id)).toEqual(["a", "b", "d"]);
    expect(filterGraph(graph, { query: "", tags: ["desktop", "capture"] }).nodes.map((n) => n.id)).toEqual(["a"]);
  });

  it("is the identity with no filter", () => {
    expect(filterGraph(graph, { query: "  ", tags: [] })).toEqual(graph);
  });
});

describe("fromElkLayout", () => {
  it("routes genealogy child → parent, draws each contradicts pair once, and marks reopeners", async () => {
    const laid = await layoutGraph(toElkGraph(graph));
    const layout = fromElkLayout(laid, graph);

    expect(layout.nodes).toHaveLength(6);
    expect(layout.width).toBeGreaterThan(0);
    expect(layout.height).toBeGreaterThan(0);
    const at = new Map(layout.nodes.map((n) => [n.id, n]));
    // DOWN: the parent's card ends above the child's card starts.
    expect(at.get("root")!.y + NODE_HEIGHT).toBeLessThan(at.get("a")!.y);
    expect(at.get("a")!.y + NODE_HEIGHT).toBeLessThan(at.get("b")!.y);
    expect(at.get("d")!.reopens).toBe(true);
    expect(at.get("a")!.reopens).toBe(false);

    const gen = layout.edges.filter((e) => e.type !== "contradicts");
    expect(gen).toHaveLength(4);
    for (const e of gen) {
      const first = e.points[0]!;
      const last = e.points[e.points.length - 1]!;
      // Starts at the child, ends at the parent, where the arrowhead goes.
      expect(first.y).toBeGreaterThan(last.y);
    }
    const con = layout.edges.filter((e) => e.type === "contradicts");
    expect(con).toHaveLength(1);
    expect(con[0]!.points).toHaveLength(2);
  });

  it("falls back to centre lines for an edge elk did not route", () => {
    const layout = fromElkLayout({ id: "root", children: [{ id: "a", x: 0, y: 0, width: 10, height: 10 }, { id: "root", x: 0, y: 100, width: 10, height: 10 }] }, graph);
    const e = layout.edges.find((x) => x.from === "a" && x.to === "root")!;
    expect(e.points).toEqual([
      { x: 5, y: 5 },
      { x: 5, y: 105 },
    ]);
  });
});

describe("lineage", () => {
  it("walks genealogy both ways and ignores contradicts", () => {
    const lin = lineage(graph.edges, "a");
    expect([...lin.ancestors]).toEqual(["root"]);
    expect([...lin.descendants]).toEqual(["b"]);
    const leaf = lineage(graph.edges, "b");
    expect([...leaf.ancestors].sort()).toEqual(["a", "root"]);
    expect(leaf.descendants.size).toBe(0);
  });
});

describe("tagCounts and truncate", () => {
  it("counts tags most common first", () => {
    expect(tagCounts(graph.nodes)).toEqual([
      { tag: "capture", count: 3 },
      { tag: "desktop", count: 3 },
    ]);
  });

  it("cuts long titles with an ellipsis", () => {
    expect(truncate("short")).toBe("short");
    const long = "a".repeat(39) + " tail that runs on";
    expect(truncate(long)).toHaveLength(40);
    expect(truncate(long).endsWith("…")).toBe(true);
  });
});

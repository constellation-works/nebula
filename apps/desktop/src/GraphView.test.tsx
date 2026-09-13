import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { GraphView } from "./GraphView";
import type { GraphExport } from "./types/GraphExport";
import type { NodeSummary } from "./types/NodeSummary";

vi.mock("./api");

const mocked = vi.mocked(api);

const statuses: NodeSummary["status"][] = ["seed", "hypothesis", "refuted", "abandoned"];

/**
 * A corpus-shaped DAG: node i derives from a node before it (a tree with the
 * odd diamond), one contradicts pair per twenty nodes, a tag per five.
 */
function synthetic(n: number): GraphExport {
  const nodes: NodeSummary[] = [];
  const edges: GraphExport["edges"] = [];
  for (let i = 0; i < n; i++) {
    nodes.push({
      id: `n${i}`,
      title: `Idea number ${i}, stated at some length so the card has to cut it`,
      status: statuses[i % 4]!,
      tags: [`t${i % 5}`, ...(i % 7 === 0 ? ["seven", "extra", "more", "yet-more"] : [])],
      created: "2026-09-01",
      updated: "2026-09-10",
    });
    if (i > 0) {
      edges.push({ from: `n${i}`, type: i % 3 === 0 ? "refines" : "derives-from", to: `n${Math.floor((i - 1) / 2)}` });
      if (i % 11 === 0 && i > 2) edges.push({ from: `n${i}`, type: "generalizes", to: `n${i - 2}` });
    }
    if (i % 20 === 19) {
      edges.push({ from: `n${i}`, type: "contradicts", to: `n${i - 1}` });
      edges.push({ from: `n${i - 1}`, type: "contradicts", to: `n${i}` });
    }
  }
  return { nodes, edges };
}

const canvas = () => screen.getByRole("img", { name: "Graph" });
const drawnNodes = () => canvas().querySelectorAll("g.node");

beforeEach(() => {
  vi.resetAllMocks();
  mocked.onCorpusChanged.mockResolvedValue(() => {});
  mocked.openInEditor.mockResolvedValue(undefined);
  mocked.node.mockImplementation(async (id) => ({
    node: { id, title: `Idea ${id}`, status: "seed", created: "", updated: "" },
    body: "",
  }));
});

describe("GraphView", () => {
  it("draws every node and edge from one graph() call", async () => {
    const g = synthetic(12);
    mocked.graph.mockResolvedValue(g);
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(12));
    expect(mocked.graph).toHaveBeenCalledTimes(1);
    // The mutual contradicts pair is drawn once, dashed; genealogy has arrowheads.
    const paths = canvas().querySelectorAll("path.edge");
    const gen = g.edges.filter((e) => e.type !== "contradicts").length;
    expect(paths).toHaveLength(gen);
    expect(canvas().querySelectorAll("path.edge--contradicts")).toHaveLength(0);
    expect(canvas().querySelector("path.edge--genealogy")).toHaveAttribute("marker-end", "url(#arrow)");
    // Status → class; the card carries the full title for hover.
    expect(canvas().querySelector('g.node[data-id="n1"]')).toHaveClass("node--hypothesis");
    const heavy = canvas().querySelector('g.node[data-id="n0"]')!;
    expect(heavy.querySelector("title")).toHaveTextContent(g.nodes[0]!.title);
    expect(heavy.querySelector(".node__title")).toHaveTextContent(/…$/);
    // Three chips then "+n" for the tag-heavy node.
    expect(heavy.querySelectorAll("g.chip")).toHaveLength(3);
    expect(heavy.querySelector(".node__more")).toHaveTextContent("+2");
    expect(screen.getByText("12 nodes", { exact: false })).toBeInTheDocument();
  });

  it("draws contradicts once and dashed", async () => {
    mocked.graph.mockResolvedValue(synthetic(20));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(20));
    const con = canvas().querySelectorAll("path.edge--contradicts");
    expect(con).toHaveLength(1);
    expect(con[0]).not.toHaveAttribute("marker-end");
  });

  it("selects on click, tints lineage, opens the panel, clears on Escape", async () => {
    mocked.graph.mockResolvedValue(synthetic(8));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(8));
    // n3 derives from n1, which derives from n0; n7 derives from n3.
    fireEvent.click(canvas().querySelector('g.node[data-id="n3"]')!);
    expect(await screen.findByRole("complementary", { name: "Node" })).toBeInTheDocument();
    await waitFor(() => expect(mocked.node).toHaveBeenCalledWith("n3"));
    expect(canvas().querySelector('g.node[data-id="n3"]')).toHaveClass("node--selected");
    expect(canvas().querySelector('g.node[data-id="n1"]')).toHaveClass("node--ancestor");
    expect(canvas().querySelector('g.node[data-id="n0"]')).toHaveClass("node--ancestor");
    expect(canvas().querySelector('g.node[data-id="n7"]')).toHaveClass("node--descendant");
    expect(canvas().querySelector('g.node[data-id="n2"]')).toHaveClass("node--dim");
    expect(canvas().querySelectorAll("path.edge--ancestor")).toHaveLength(2);
    expect(canvas().querySelectorAll("path.edge--descendant")).toHaveLength(1);

    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Node" })).toBeNull());
    expect(canvas().querySelector("g.node--dim")).toBeNull();
  });

  it("double-click opens the file in the editor", async () => {
    mocked.graph.mockResolvedValue(synthetic(3));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(3));
    fireEvent.doubleClick(canvas().querySelector('g.node[data-id="n2"]')!);
    expect(mocked.openInEditor).toHaveBeenCalledWith("n2");
  });

  it("filters by title and by tag before layout, and says when nothing matches", async () => {
    mocked.graph.mockResolvedValue(synthetic(10));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(10));

    fireEvent.change(screen.getByLabelText("Filter by title"), { target: { value: "number 1" } });
    await waitFor(() => expect(drawnNodes()).toHaveLength(1));
    expect(screen.getByText("1 of 10 nodes", { exact: false })).toBeInTheDocument();
    // n1's only edge went to n0, which is hidden, so no edge is drawn.
    expect(canvas().querySelectorAll("path.edge")).toHaveLength(0);

    fireEvent.change(screen.getByLabelText("Filter by title"), { target: { value: "" } });
    await waitFor(() => expect(drawnNodes()).toHaveLength(10));
    fireEvent.click(within(screen.getByRole("list", { name: "Filter by tag" })).getByRole("button", { name: /^t2/ }));
    await waitFor(() => expect(drawnNodes()).toHaveLength(2));
    fireEvent.click(within(screen.getByRole("list", { name: "Filter by tag" })).getByRole("button", { name: /^t3/ }));
    expect(await screen.findByText("No nodes match the filter.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await waitFor(() => expect(drawnNodes()).toHaveLength(10));
  });

  it("refetches on corpus-changed and keeps the selection", async () => {
    let fire: () => void = () => {};
    mocked.onCorpusChanged.mockImplementation(async (handler) => {
      fire = handler;
      return () => {};
    });
    mocked.graph.mockResolvedValue(synthetic(5));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(5));
    fireEvent.click(canvas().querySelector('g.node[data-id="n4"]')!);
    await screen.findByRole("complementary", { name: "Node" });

    mocked.graph.mockResolvedValue(synthetic(6));
    act(() => fire());
    await waitFor(() => expect(drawnNodes()).toHaveLength(6));
    expect(mocked.graph).toHaveBeenCalledTimes(2);
    expect(canvas().querySelector('g.node[data-id="n4"]')).toHaveClass("node--selected");
    expect(screen.getByRole("complementary", { name: "Node" })).toBeInTheDocument();
    // The panel re-reads its node, in case the body changed.
    expect(mocked.node).toHaveBeenCalledTimes(2);
  });

  it("says what to do when the corpus has no nodes", async () => {
    mocked.graph.mockResolvedValue({ nodes: [], edges: [] });
    render(<GraphView />);
    expect(await screen.findByText(/Capture something, then promote it with the agent/)).toBeInTheDocument();
  });

  it("lays out and draws 200 nodes", async () => {
    mocked.graph.mockResolvedValue(synthetic(200));
    const t0 = performance.now();
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(200), { timeout: 10_000 });
    const ms = performance.now() - t0;
    // jsdom is slower than the webview; this is a ceiling, not the number.
    console.info(`200 nodes: layout + render in jsdom took ${Math.round(ms)} ms`);
    expect(ms).toBeLessThan(5000);
  });
});

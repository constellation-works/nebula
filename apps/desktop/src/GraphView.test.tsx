import { act, fireEvent, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { GraphView } from "./GraphView";
import * as layoutClient from "./layoutClient";
import { useGraph } from "./useGraph";
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const canvas = () => screen.getByRole("group", { name: "Graph" });
const drawnNodes = () => canvas().querySelectorAll("g.node");

beforeEach(() => {
  vi.resetAllMocks();
  mocked.onCorpusChanged.mockResolvedValue(() => {});
  mocked.openInEditor.mockResolvedValue(undefined);
  mocked.reload.mockResolvedValue(undefined);
  mocked.captureShortcut.mockResolvedValue("CmdOrCtrl+Shift+N");
  mocked.graphSearch.mockResolvedValue([]);
  mocked.node.mockImplementation(async (id) => ({
    node: { id, title: `Idea ${id}`, status: "seed", created: "", updated: "" },
    body: "",
  }));
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
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

  it("tabs into a card, follows an edge with arrows, and selects with Enter", async () => {
    const graph = synthetic(2);
    graph.nodes.push({ ...graph.nodes[0]!, id: "isolated", title: "Standalone idea" });
    mocked.graph.mockResolvedValue(graph);
    render(<GraphView />);
    const first = await screen.findByRole("button", { name: /Idea number 0.*seed/ });
    const child = screen.getByRole("button", { name: /Idea number 1.*hypothesis/ });
    const isolated = screen.getByRole("button", { name: "Standalone idea, seed" });
    expect(first).toHaveAttribute("tabindex", "0");
    expect(child).toHaveAttribute("tabindex", "0");
    expect(isolated).toHaveAttribute("tabindex", "0");
    const before = canvas().querySelector("g.scene")!.getAttribute("transform");
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowDown" });
    expect(child).toHaveFocus();
    expect(canvas().querySelector("g.scene")!.getAttribute("transform")).not.toBe(before);
    fireEvent.keyDown(child, { key: "Enter" });
    expect(child).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByRole("complementary", { name: "Node" })).toBeInTheDocument();
    fireEvent.keyDown(child, { key: "ArrowUp" });
    expect(first).toHaveFocus();
    isolated.focus();
    fireEvent.keyDown(isolated, { key: "ArrowDown" });
    expect(isolated).toHaveFocus();
  });

  it("double-click opens the file in the editor", async () => {
    mocked.graph.mockResolvedValue(synthetic(3));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(3));
    fireEvent.doubleClick(canvas().querySelector('g.node[data-id="n2"]')!);
    expect(mocked.openInEditor).toHaveBeenCalledWith("n2");
  });

  it("shows a dismissible error when double-click cannot open the file", async () => {
    mocked.graph.mockResolvedValue(synthetic(1));
    mocked.openInEditor.mockRejectedValueOnce({ code: "open_failed", message: "no .md handler" });
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(1));
    fireEvent.doubleClick(canvas().querySelector('g.node[data-id="n0"]')!);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not open file: no .md handler");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss open error" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows loading, then offers graph error guidance and Reload", async () => {
    const request = deferred<GraphExport>();
    mocked.graph.mockReturnValueOnce(request.promise).mockResolvedValueOnce(synthetic(2));
    render(<GraphView />);
    expect(screen.getByRole("status")).toHaveTextContent("Loading graph");
    await act(async () => request.reject({ code: "yaml", message: "bad node frontmatter" }));
    expect(screen.getByRole("alert")).toHaveTextContent("bad node frontmatter");
    expect(screen.getByRole("alert")).toHaveTextContent("neb check");
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    await waitFor(() => expect(drawnNodes()).toHaveLength(2));
    expect(mocked.reload).toHaveBeenCalledTimes(1);
    expect(mocked.graph).toHaveBeenCalledTimes(2);
  });

  it("dims nonmatches without losing edges, then steps through matches and focuses each", async () => {
    mocked.graph.mockResolvedValue(synthetic(10));
    mocked.graphSearch.mockResolvedValue(["n1", "n3"]);
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(10));
    const edges = canvas().querySelectorAll("path.edge").length;
    const initial = canvas().querySelector("g.scene")!.getAttribute("transform");
    fireEvent.change(screen.getByRole("searchbox", { name: "Search graph" }), { target: { value: "evidence" } });
    await waitFor(() => expect(mocked.graphSearch).toHaveBeenCalledWith("evidence"));
    await waitFor(() => expect(screen.getByText("2 of 10 nodes", { exact: false })).toBeInTheDocument());
    expect(drawnNodes()).toHaveLength(10);
    expect(canvas().querySelectorAll("path.edge")).toHaveLength(edges);
    expect(canvas().querySelector('g.node[data-id="n0"]')).toHaveClass("node--filter-dim");
    expect(canvas().querySelector('g.node[data-id="n1"]')).not.toHaveClass("node--filter-dim");
    fireEvent.click(screen.getByRole("button", { name: "Next match" }));
    expect(canvas().querySelector('g.node[data-id="n1"]')).toHaveClass("node--selected");
    const first = canvas().querySelector("g.scene")!.getAttribute("transform");
    expect(first).not.toBe(initial);
    fireEvent.click(screen.getByRole("button", { name: "Next match" }));
    expect(canvas().querySelector('g.node[data-id="n3"]')).toHaveClass("node--selected");
    expect(canvas().querySelector("g.scene")!.getAttribute("transform")).not.toBe(first);
    fireEvent.click(screen.getByRole("button", { name: "Previous match" }));
    expect(canvas().querySelector('g.node[data-id="n1"]')).toHaveClass("node--selected");
  });

  it("combines tags with search and retains the canvas when no node matches", async () => {
    mocked.graph.mockResolvedValue(synthetic(10));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(10));
    fireEvent.click(within(screen.getByRole("list", { name: "Filter by tag" })).getByRole("button", { name: /^t2/ }));
    expect(canvas().querySelectorAll("g.node--filter-dim")).toHaveLength(8);
    fireEvent.click(within(screen.getByRole("list", { name: "Filter by tag" })).getByRole("button", { name: /^t3/ }));
    expect(await screen.findByText("No nodes match the filter.")).toBeInTheDocument();
    expect(drawnNodes()).toHaveLength(10);
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(canvas().querySelector("g.node--filter-dim")).toBeNull();
  });

  it("debounces search and does not re-layout on filter changes", async () => {
    mocked.graph.mockResolvedValue(synthetic(10));
    const layoutSpy = vi.spyOn(layoutClient, "layoutGraph");
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(10));
    expect(layoutSpy).toHaveBeenCalledTimes(1);
    const input = screen.getByRole("searchbox", { name: "Search graph" });
    fireEvent.change(input, { target: { value: "number" } });
    fireEvent.change(input, { target: { value: "number 1" } });
    await waitFor(() => expect(mocked.graphSearch).toHaveBeenCalledWith("number 1"));
    expect(mocked.graphSearch).toHaveBeenCalledTimes(1);
    expect(layoutSpy).toHaveBeenCalledTimes(1);
    expect(drawnNodes()).toHaveLength(10);
  });

  it("isolates the selected node with ancestors and descendants at their original positions", async () => {
    mocked.graph.mockResolvedValue(synthetic(8));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(8));
    const before = canvas().querySelector('g.node[data-id="n3"]')!.getAttribute("transform");
    fireEvent.click(canvas().querySelector('g.node[data-id="n3"]')!);
    fireEvent.click(screen.getByRole("button", { name: "Lineage only" }));
    expect([...drawnNodes()].map((n) => n.getAttribute("data-id"))).toEqual(["n0", "n1", "n3", "n7"]);
    expect(canvas().querySelector('g.node[data-id="n3"]')).toHaveAttribute("transform", before);
    expect(canvas().querySelectorAll("path.edge")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Lineage only" }));
    expect(drawnNodes()).toHaveLength(8);
  });

  it("shows graph interactions and the configured capture shortcut", async () => {
    mocked.graph.mockResolvedValue(synthetic(2));
    render(<GraphView />);
    expect(await screen.findByText(/Capture: CmdOrCtrl\+Shift\+N/)).toHaveTextContent("Double-click a node to open");
    expect(screen.getByText(/Orange ancestors/)).toHaveTextContent("Ctrl+scroll to zoom");
    expect(screen.getByText(/Orange ancestors/)).toHaveTextContent("arrow keys follow edges, Enter selects");
  });

  it("pans on plain wheel and zooms around the pointer on ctrl-wheel", async () => {
    mocked.graph.mockResolvedValue(synthetic(2));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(2));
    const scene = canvas().querySelector("g.scene")!;
    expect(scene).toHaveAttribute("transform", "translate(40 40) scale(1)");

    const scroll = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaX: 12, deltaY: 30 });
    fireEvent(canvas(), scroll);
    expect(scroll.defaultPrevented).toBe(true);
    expect(scene).toHaveAttribute("transform", "translate(28 10) scale(1)");

    const pinch = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      ctrlKey: true,
      clientX: 100,
      clientY: 100,
      deltaY: -100,
    });
    fireEvent(canvas(), pinch);
    expect(pinch.defaultPrevented).toBe(true);
    const zoom = Math.exp(0.25);
    expect(scene).toHaveAttribute(
      "transform",
      `translate(${100 - (100 - 28) * zoom} ${100 - (100 - 10) * zoom}) scale(${zoom})`,
    );
  });

  it("converts line wheel deltas and clamps pointer-centered zoom", async () => {
    mocked.graph.mockResolvedValue(synthetic(2));
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(2));
    const svg = canvas();
    const scene = svg.querySelector("g.scene")!;
    vi.spyOn(svg, "getBoundingClientRect").mockReturnValue({ left: 20, top: 30 } as DOMRect);

    fireEvent.wheel(svg, { deltaMode: 1, deltaX: 2, deltaY: 3 });
    expect(scene).toHaveAttribute("transform", "translate(8 -8) scale(1)");
    fireEvent.wheel(svg, { ctrlKey: true, clientX: 120, clientY: 130, deltaY: -10000 });
    expect(scene).toHaveAttribute("transform", "translate(-176 -224) scale(3)");
    fireEvent.wheel(svg, { ctrlKey: true, clientX: 120, clientY: 130, deltaY: 10000 });
    expect(scene).toHaveAttribute("transform", "translate(86.2 83.8) scale(0.15)");
  });

  it("ignores small drags, pans past the threshold, and fits the laid out graph", async () => {
    mocked.graph.mockResolvedValue(synthetic(4));
    const layoutSpy = vi.spyOn(layoutClient, "layoutGraph");
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(4));
    const svg = canvas();
    const scene = svg.querySelector("g.scene")!;
    Object.defineProperties(svg, {
      clientWidth: { configurable: true, value: 400 },
      clientHeight: { configurable: true, value: 300 },
    });
    fireEvent.click(svg.querySelector('g.node[data-id="n2"]')!);
    expect(svg.querySelector('g.node[data-id="n2"]')).toHaveClass("node--selected");

    fireEvent.mouseDown(svg, { button: 0, clientX: 50, clientY: 50 });
    fireEvent.mouseMove(svg, { clientX: 52, clientY: 52 });
    expect(scene).toHaveAttribute("transform", "translate(40 40) scale(1)");
    fireEvent.mouseMove(svg, { clientX: 75, clientY: 35 });
    expect(scene).toHaveAttribute("transform", "translate(65 25) scale(1)");
    fireEvent.mouseUp(svg, { clientX: 75, clientY: 35 });
    expect(svg.querySelector('g.node[data-id="n2"]')).toHaveClass("node--selected");

    const laid = await layoutSpy.mock.results[0]!.value;
    const k = Math.min(1, 320 / laid.width!, 220 / laid.height!);
    fireEvent.click(screen.getByRole("button", { name: "Fit" }));
    expect(scene).toHaveAttribute(
      "transform",
      `translate(${(400 - laid.width! * k) / 2} ${(300 - laid.height! * k) / 2}) scale(${k})`,
    );
  });

  it("fits the first layout in the commit that draws it", async () => {
    vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(400);
    vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(300);
    mocked.graph.mockResolvedValue(synthetic(4));
    const layoutSpy = vi.spyOn(layoutClient, "layoutGraph");
    render(<GraphView />);
    // waitFor's MutationObserver checks right after the commit that draws the
    // nodes, before any later task. A fit deferred past that commit shows an
    // unfitted frame here, and a wheel handled in the gap was overwritten.
    let first: string | null = null;
    await waitFor(() => {
      expect(drawnNodes()).toHaveLength(4);
      first = canvas().querySelector("g.scene")!.getAttribute("transform");
    });
    const laid = await layoutSpy.mock.results[0]!.value;
    const k = Math.min(1, 320 / laid.width!, 220 / laid.height!);
    const fitted = `translate(${(400 - laid.width! * k) / 2} ${(300 - laid.height! * k) / 2}) scale(${k})`;
    expect(first).toBe(fitted);
    expect(canvas().querySelector("g.scene")).toHaveAttribute("transform", fitted);
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

    fireEvent.wheel(canvas(), { deltaX: 12, deltaY: 30 });
    const viewport = canvas().querySelector("g.scene")!.getAttribute("transform");
    expect(viewport).toBe("translate(28 10) scale(1)");

    mocked.graph.mockResolvedValue(synthetic(6));
    act(() => fire());
    await waitFor(() => expect(drawnNodes()).toHaveLength(6));
    expect(mocked.graph).toHaveBeenCalledTimes(2);
    expect(canvas().querySelector('g.node[data-id="n4"]')).toHaveClass("node--selected");
    expect(canvas().querySelector("g.scene")).toHaveAttribute("transform", viewport);
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

  it("lays out 500 nodes and preserves their positions on refetch", async () => {
    let fire: () => void = () => {};
    mocked.onCorpusChanged.mockImplementation(async (handler) => {
      fire = handler;
      return () => {};
    });
    mocked.graph.mockResolvedValueOnce(synthetic(500)).mockResolvedValueOnce(synthetic(500));
    const layoutSpy = vi.spyOn(layoutClient, "layoutGraph");
    render(<GraphView />);
    await waitFor(() => expect(drawnNodes()).toHaveLength(500), { timeout: 20_000 });
    const positions = [...drawnNodes()].map((node) => [node.getAttribute("data-id"), node.getAttribute("transform")]);
    expect(layoutSpy).toHaveBeenCalledTimes(1);

    act(() => fire());
    await waitFor(() => expect(layoutSpy).toHaveBeenCalledTimes(2));
    await act(async () => { await layoutSpy.mock.results[1]!.value; });
    expect(drawnNodes()).toHaveLength(500);
    expect([...drawnNodes()].map((node) => [node.getAttribute("data-id"), node.getAttribute("transform")])).toEqual(positions);
  });
});

describe("useGraph", () => {
  it("keeps a newer snapshot when an older success settles last", async () => {
    const olderRequest = deferred<GraphExport>();
    const newerRequest = deferred<GraphExport>();
    const newer = synthetic(2);
    mocked.graph.mockReturnValueOnce(olderRequest.promise).mockReturnValueOnce(newerRequest.promise);
    const { result } = renderHook(() => useGraph());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(1));

    act(() => void result.current.refresh());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(2));
    await act(async () => {
      newerRequest.resolve(newer);
      await newerRequest.promise;
    });
    expect(result.current.graph).toEqual(newer);

    await act(async () => {
      olderRequest.resolve(synthetic(0));
      await olderRequest.promise;
    });
    expect(result.current.graph).toEqual(newer);
    expect(result.current.error).toBeNull();
  });

  it("keeps a newer snapshot when an older failure settles last", async () => {
    const olderRequest = deferred<GraphExport>();
    const newerRequest = deferred<GraphExport>();
    const newer = synthetic(2);
    mocked.graph.mockReturnValueOnce(olderRequest.promise).mockReturnValueOnce(newerRequest.promise);
    const { result } = renderHook(() => useGraph());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(1));

    act(() => void result.current.refresh());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(2));
    await act(async () => {
      newerRequest.resolve(newer);
      await newerRequest.promise;
    });

    await act(async () => {
      olderRequest.reject(new Error("older failure"));
      await olderRequest.promise.catch(() => undefined);
    });
    expect(result.current.graph).toEqual(newer);
    expect(result.current.error).toBeNull();
  });

  it("keeps a newer error when an older success settles last", async () => {
    const olderRequest = deferred<GraphExport>();
    const newerRequest = deferred<GraphExport>();
    mocked.graph.mockReturnValueOnce(olderRequest.promise).mockReturnValueOnce(newerRequest.promise);
    const { result } = renderHook(() => useGraph());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(1));

    act(() => void result.current.refresh());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(2));
    await act(async () => {
      newerRequest.reject(new Error("newer failure"));
      await newerRequest.promise.catch(() => undefined);
    });
    expect(result.current.error).toBe("Error: newer failure");

    await act(async () => {
      olderRequest.resolve(synthetic(1));
      await olderRequest.promise;
    });
    expect(result.current.graph).toBeNull();
    expect(result.current.error).toBe("Error: newer failure");
  });

  it("does not install a pending request after unmount and disposes its listener", async () => {
    const request = deferred<GraphExport>();
    const off = vi.fn();
    mocked.graph.mockReturnValueOnce(request.promise);
    mocked.onCorpusChanged.mockResolvedValueOnce(off);
    const { result, unmount } = renderHook(() => useGraph());
    await waitFor(() => expect(mocked.graph).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocked.onCorpusChanged).toHaveBeenCalledTimes(1));

    unmount();
    expect(off).toHaveBeenCalledTimes(1);
    await act(async () => {
      request.resolve(synthetic(1));
      await request.promise;
    });
    expect(result.current.graph).toBeNull();
    expect(result.current.loaded).toBe(false);
  });
});

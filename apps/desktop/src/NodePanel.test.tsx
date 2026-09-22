import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { NodePanel } from "./NodePanel";
import type { NodeSummary } from "./types/NodeSummary";
import type { NodeView } from "./types/NodeView";

vi.mock("./api");

const mocked = vi.mocked(api);

const fixture: NodeView = {
  node: {
    id: "tray-count-is-the-status-bar",
    title: "The tray count is the whole status bar",
    title_by: "agent:crew-title",
    status: "refuted",
    created: "2026-09-01",
    updated: "2026-09-12",
    kill: "A user asks for a second number in the tray.",
    kill_by: "agent:crew-alpha",
    tags: ["desktop", "capture"],
    edges: [
      { type: "derives-from", to: "capture-under-five-seconds", by: "agent:crew-genealogy" },
      { type: "contradicts", to: "dashboards-earn-their-keep", by: "agent:crew-contradiction" },
    ],
    references: [
      {
        id: "r1",
        kind: "article",
        uri: "https://example.org/menu-bar-apps",
        title: "Menu-bar apps, a survey",
        note: "the one-number precedent",
        added: "2026-09-02",
      },
      {
        id: "r2",
        kind: "note",
        uri: "almanac/notes/tray.md",
        title: null,
        note: null,
        added: "2026-09-03",
      },
      {
        id: "r3",
        kind: "discussion",
        uri: null,
        title: "Team debrief",
        note: null,
        added: "2026-09-04",
      },
      {
        id: "r4",
        kind: "observatory",
        uri: "Q002",
        title: null,
        note: "the question this became",
        added: "2026-09-05",
        by: "agent:crew-reference",
      },
      {
        id: "r5",
        kind: "observatory",
        uri: "H007",
        title: null,
        note: null,
        added: "2026-09-06",
      },
    ],
    closed: { why: "Two users asked for the review count too.", at: "2026-09-12" },
  },
  body: [
    "## Argument",
    "",
    "One number is **enough** for a glance. See [the survey](https://example.org/menu-bar-apps).",
    "",
    "<script>alert('no')</script><b>raw html stays text</b>",
    "",
    "| a | b |",
    "|---|---|",
    "| 1 | 2 |",
  ].join("\n"),
  observatory: [
    { reference: "r4", record: "Q002", path: "/research/observatory/questions/Q002.md" },
    { reference: "r5", record: "H007", path: null },
  ],
};

const nodes: NodeSummary[] = [
  { id: "capture-under-five-seconds", title: "Capture must stay under five seconds", status: "hypothesis", tags: [], created: "", updated: "" },
  { id: "dashboards-earn-their-keep", title: "Dashboards earn their keep", status: "seed", tags: [], created: "", updated: "" },
];

function nodeView(id: string, title: string, body: string): NodeView {
  return {
    ...fixture,
    node: { ...fixture.node, id, title },
    body,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.resetAllMocks();
  mocked.node.mockResolvedValue(fixture);
  mocked.openUrl.mockResolvedValue(undefined);
  mocked.openInEditor.mockResolvedValue(undefined);
});

function renderPanel(onSelect = vi.fn(), onClose = vi.fn()) {
  render(
    <NodePanel
      id={fixture.node.id}
      nodes={nodes}
      revision={1}
      width={380}
      onResize={() => {}}
      onSelect={onSelect}
      onClose={onClose}
    />,
  );
  return { onSelect, onClose };
}

describe("NodePanel", () => {
  it("shows the frontmatter: title, status, kill, closed.why, tags", async () => {
    renderPanel();
    expect(await screen.findByRole("heading", { level: 2, name: fixture.node.title })).toBeInTheDocument();
    expect(mocked.node).toHaveBeenCalledWith(fixture.node.id);
    expect(screen.getByText("refuted")).toHaveClass("badge--refuted");
    expect(screen.getByText("A user asks for a second number in the tray.")).toBeInTheDocument();
    expect(screen.getByText("Closed 2026-09-12")).toBeInTheDocument();
    expect(screen.getByText("Two users asked for the review count too.")).toBeInTheDocument();
    const tags = within(screen.getByRole("list", { name: "Tags" })).getAllByRole("listitem");
    expect(tags.map((t) => t.textContent)).toEqual(["desktop", "capture"]);
  });

  it("shows non-human authorship and resolves observatory references", async () => {
    renderPanel();
    await screen.findByRole("heading", { level: 2, name: fixture.node.title });
    expect(screen.getByText("by agent:crew-title")).toBeInTheDocument();
    expect(screen.getByText("by agent:crew-alpha")).toBeInTheDocument();
    expect(screen.getByText("by agent:crew-genealogy")).toBeInTheDocument();
    expect(screen.getByText("by agent:crew-contradiction")).toBeInTheDocument();
    expect(screen.getByText("by agent:crew-reference")).toBeInTheDocument();
    expect(screen.getByText("/research/observatory/questions/Q002.md")).toBeInTheDocument();
    expect(screen.getByText("(does not resolve on this machine)")).toBeInTheDocument();
  });

  it("renders the body as markdown with GFM and without raw HTML", async () => {
    renderPanel();
    expect(await screen.findByRole("heading", { level: 2, name: "Argument" })).toBeInTheDocument();
    expect(screen.getByText("enough").tagName).toBe("STRONG");
    // GFM: the pipe table becomes a table.
    expect(screen.getByRole("cell", { name: "1" })).toBeInTheDocument();
    // Raw HTML is escaped, never executed or rendered.
    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector(".markdown b")).toBeNull();
    expect(screen.getByText(/raw html stays text/)).toHaveTextContent("<b>raw html stays text</b>");
    // A link in the body leaves through the opener, not the webview.
    const link = screen.getByRole("link", { name: "the survey" });
    fireEvent.click(link);
    await waitFor(() => expect(mocked.openUrl).toHaveBeenCalledWith("https://example.org/menu-bar-apps"));
  });

  it("lists edges under genealogy and contradicts, each selecting its node", async () => {
    const { onSelect } = renderPanel();
    await screen.findByRole("heading", { level: 3, name: "Genealogy" });
    expect(screen.getByRole("heading", { level: 3, name: "Contradicts" })).toBeInTheDocument();
    expect(screen.getByText("derives-from")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Capture must stay under five seconds" }));
    expect(onSelect).toHaveBeenCalledWith("capture-under-five-seconds");
    fireEvent.click(screen.getByRole("button", { name: "Dashboards earn their keep" }));
    expect(onSelect).toHaveBeenCalledWith("dashboards-earn-their-keep");
  });

  it("tables the references and opens external uris through the opener", async () => {
    renderPanel();
    // The body's GFM table is also a table; the references one follows its heading.
    const heading = await screen.findByRole("heading", { level: 3, name: "References" });
    const rows = within(heading.parentElement!).getAllByRole("row");
    expect(rows).toHaveLength(6);
    expect(rows[1]).toHaveTextContent("article");
    expect(rows[1]).toHaveTextContent("the one-number precedent");
    expect(rows[1]).toHaveTextContent("2026-09-02");
    fireEvent.click(within(rows[1]!).getByRole("link", { name: "Menu-bar apps, a survey" }));
    await waitFor(() => expect(mocked.openUrl).toHaveBeenCalledWith("https://example.org/menu-bar-apps"));
    // A local path is not a link: nothing to hand to the OS.
    expect(within(rows[2]!).queryByRole("link")).toBeNull();
    expect(within(rows[2]!).getByText("almanac/notes/tray.md").tagName).toBe("CODE");
    // A uri-less reference (e.g. a discussion) renders as plain text, not a link.
    expect(within(rows[3]!).queryByRole("link")).toBeNull();
    expect(within(rows[3]!).getByText("Team debrief")).toBeInTheDocument();
    expect(within(rows[4]!).getByText("Q002").tagName).toBe("CODE");
    expect(within(rows[4]!).getByText("/research/observatory/questions/Q002.md").tagName).toBe("CODE");
  });

  it("opens the file and closes on request", async () => {
    const { onClose } = renderPanel();
    await screen.findByRole("heading", { level: 2, name: fixture.node.title });
    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    expect(mocked.openInEditor).toHaveBeenCalledWith(fixture.node.id);
    fireEvent.click(screen.getByRole("button", { name: "Close panel" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("shows the backend's message when the node cannot be read", async () => {
    mocked.node.mockRejectedValue("no such node: gone");
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent("no such node: gone");
  });

  it("hides the prior node while a new selection loads and after it fails", async () => {
    const first = nodeView("first", "First node", "Body from first");
    const second = deferred<NodeView>();
    mocked.node.mockResolvedValueOnce(first).mockReturnValueOnce(second.promise);

    const { rerender } = render(
      <NodePanel
        id="first"
        nodes={nodes}
        revision={1}
        width={380}
        onResize={() => {}}
        onSelect={() => {}}
        onClose={() => {}}
      />,
    );
    expect(await screen.findByText("Body from first")).toBeInTheDocument();

    rerender(
      <NodePanel
        id="second"
        nodes={nodes}
        revision={1}
        width={380}
        onResize={() => {}}
        onSelect={() => {}}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("second")).toHaveClass("panel__id");
    expect(screen.queryByText("First node")).toBeNull();
    expect(screen.queryByText("Body from first")).toBeNull();

    await act(async () => second.reject("no such node: second"));
    expect(await screen.findByRole("alert")).toHaveTextContent("no such node: second");
    expect(screen.queryByText("First node")).toBeNull();
    expect(screen.queryByText("Body from first")).toBeNull();
  });

  it("keeps only the current node when requests resolve out of order", async () => {
    const first = deferred<NodeView>();
    const second = deferred<NodeView>();
    mocked.node.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const { rerender } = render(
      <NodePanel
        id="first"
        nodes={nodes}
        revision={1}
        width={380}
        onResize={() => {}}
        onSelect={() => {}}
        onClose={() => {}}
      />,
    );
    rerender(
      <NodePanel
        id="second"
        nodes={nodes}
        revision={1}
        width={380}
        onResize={() => {}}
        onSelect={() => {}}
        onClose={() => {}}
      />,
    );

    await act(async () => second.resolve(nodeView("second", "Second node", "Body from second")));
    expect(await screen.findByText("Body from second")).toBeInTheDocument();

    await act(async () => first.resolve(nodeView("first", "First node", "Late body from first")));
    expect(screen.getByText("Second node")).toBeInTheDocument();
    expect(screen.getByText("Body from second")).toBeInTheDocument();
    expect(screen.queryByText("Late body from first")).toBeNull();
  });
});

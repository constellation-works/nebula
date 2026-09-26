import { act, fireEvent, render, renderHook, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { App } from "./App";
import { CaptureBox, CONFIRM_MS } from "./CaptureBox";
import { CaptureWindow } from "./CaptureWindow";
import { InboxView } from "./InboxView";
import * as layoutClient from "./layoutClient";
import { useInbox } from "./useInbox";
import type { InboxEntry } from "./types/InboxEntry";

vi.mock("./api");

const windowMock = vi.hoisted(() => ({ onFocusChanged: vi.fn(), hide: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));

const mocked = vi.mocked(api);

const entries: InboxEntry[] = [
  { id: "a1b2", at: "2026-09-13T09:00", text: "capture must stay under five seconds" },
  { id: "c3d4", at: "2026-09-13T10:30", text: "the tray count is the whole status bar" },
];

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

/** The view as the shell mounts it: over the hook, over the (mocked) api. */
function Harness() {
  return <InboxView inbox={useInbox()} />;
}

beforeEach(() => {
  vi.resetAllMocks();
  mocked.inbox.mockResolvedValue(entries);
  mocked.onCorpusChanged.mockResolvedValue(() => {});
  mocked.corpusPath.mockResolvedValue("/tmp/nowhere/.nebula");
  mocked.startupWarnings.mockResolvedValue([]);
  mocked.reload.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("InboxView", () => {
  it("renders every unsettled entry with id, age and text", async () => {
    render(<Harness />);
    expect(await screen.findByText("capture must stay under five seconds")).toBeInTheDocument();
    expect(screen.getByText("the tray count is the whole status bar")).toBeInTheDocument();
    expect(screen.getByText("a1b2")).toBeInTheDocument();
    expect(screen.getByText("c3d4")).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    // Age is derived from `at`, so the raw stamp stays available on hover.
    expect(screen.getByTitle("2026-09-13T09:00")).toBeInTheDocument();
    expect(mocked.inbox).toHaveBeenCalledTimes(1);
  });

  it("says so when nothing is waiting", async () => {
    mocked.inbox.mockResolvedValue([]);
    render(<Harness />);
    expect(await screen.findByText("Nothing waiting.")).toBeInTheDocument();
    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
  });

  it("refetches when the corpus changes on disk", async () => {
    let fire: () => void = () => {};
    mocked.onCorpusChanged.mockImplementation(async (handler) => {
      fire = handler;
      return () => {};
    });
    render(<Harness />);
    await screen.findByText("a1b2");
    mocked.inbox.mockResolvedValue([...entries, { id: "e5f6", at: "2026-09-13T11:00", text: "from neb" }]);
    act(() => fire());
    expect(await screen.findByText("from neb")).toBeInTheDocument();
    expect(mocked.inbox).toHaveBeenCalledTimes(2);
  });

  it("captures on Enter and shows the confirmation", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocked.capture.mockResolvedValue({ id: "9999", at: "2026-09-13T12:00", text: "new" });
    render(<Harness />);
    await screen.findByText("a1b2");
    const input = screen.getByLabelText("Capture");
    fireEvent.change(input, { target: { value: "  new  " } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mocked.capture).toHaveBeenCalledWith("new"));
    expect(await screen.findByText("captured")).toBeInTheDocument();
    expect(input).toHaveValue("");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1100);
    });
    expect(screen.queryByText("captured")).not.toBeInTheDocument();
    // The list is asked again once the confirmation has been shown.
    expect(mocked.inbox).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });

  it("drops one entry and refreshes the list and Inbox badge", async () => {
    mocked.inbox.mockResolvedValueOnce(entries).mockResolvedValueOnce([entries[1]]);
    mocked.dropEntry.mockResolvedValue(entries[0]);
    render(<App />);
    expect(await screen.findByRole("tab", { name: "Inbox2" })).toBeInTheDocument();

    fireEvent.click(within(screen.getAllByRole("listitem")[0]).getByRole("button", { name: "Drop" }));
    await waitFor(() => expect(mocked.dropEntry).toHaveBeenCalledWith("a1b2"));
    expect(await screen.findByRole("tab", { name: "Inbox1" })).toBeInTheDocument();
    expect(screen.queryByText("a1b2")).not.toBeInTheDocument();
    expect(screen.getByText("c3d4")).toBeInTheDocument();
  });

  it("promotes one entry as a root and refreshes the list and Inbox badge", async () => {
    mocked.inbox.mockResolvedValueOnce(entries).mockResolvedValueOnce([entries[1]]);
    mocked.promoteRoot.mockResolvedValue({ doc: { node: { id: "capture-must-stay" } } } as never);
    render(<App />);
    expect(await screen.findByRole("tab", { name: "Inbox2" })).toBeInTheDocument();

    fireEvent.click(within(screen.getAllByRole("listitem")[0]).getByRole("button", { name: "Promote as root" }));
    await waitFor(() => expect(mocked.promoteRoot).toHaveBeenCalledWith("a1b2"));
    expect(await screen.findByRole("tab", { name: "Inbox1" })).toBeInTheDocument();
    expect(screen.queryByText("a1b2")).not.toBeInTheDocument();
  });

  it("keeps a failed entry in place with its error and allows a retry", async () => {
    mocked.dropEntry.mockRejectedValueOnce("corpus busy").mockRejectedValueOnce("already dropped");
    render(<Harness />);
    await screen.findByText("a1b2");
    const first = screen.getAllByRole("listitem")[0];

    fireEvent.click(within(first).getByRole("button", { name: "Drop" }));
    expect(await within(first).findByRole("alert")).toHaveTextContent("corpus busy");
    expect(mocked.inbox).toHaveBeenCalledTimes(1);
    expect(screen.getAllByRole("listitem")).toHaveLength(2);

    fireEvent.click(within(first).getByRole("button", { name: "Drop" }));
    expect(await within(first).findByRole("alert")).toHaveTextContent("already dropped");
    expect(mocked.inbox).toHaveBeenCalledTimes(1);
  });

  it("shows the stale marker at fourteen calendar days", async () => {
    vi.spyOn(Date, "now").mockReturnValue(new Date(2026, 8, 27, 0, 1).getTime());
    mocked.inbox.mockResolvedValueOnce([entries[0], { ...entries[1], at: "2026-09-14T00:00" }]);
    render(<Harness />);
    await screen.findByText("a1b2");
    expect(within(screen.getAllByRole("listitem")[0]).getByText("stale")).toBeInTheDocument();
    expect(within(screen.getAllByRole("listitem")[1]).queryByText("stale")).not.toBeInTheDocument();
  });
});

describe("useInbox", () => {
  const newer = [{ id: "e5f6", at: "2026-09-13T11:00", text: "newer" }];

  it("keeps a newer snapshot when an older success settles last", async () => {
    const olderRequest = deferred<InboxEntry[]>();
    const newerRequest = deferred<InboxEntry[]>();
    mocked.inbox.mockReturnValueOnce(olderRequest.promise).mockReturnValueOnce(newerRequest.promise);
    const { result } = renderHook(() => useInbox());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(1));

    act(() => void result.current.refresh());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(2));
    await act(async () => {
      newerRequest.resolve(newer);
      await newerRequest.promise;
    });
    expect(result.current.entries).toEqual(newer);

    await act(async () => {
      olderRequest.resolve([]);
      await olderRequest.promise;
    });
    expect(result.current.entries).toEqual(newer);
    expect(result.current.error).toBeNull();
  });

  it("keeps a newer snapshot when an older failure settles last", async () => {
    const olderRequest = deferred<InboxEntry[]>();
    const newerRequest = deferred<InboxEntry[]>();
    mocked.inbox.mockReturnValueOnce(olderRequest.promise).mockReturnValueOnce(newerRequest.promise);
    const { result } = renderHook(() => useInbox());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(1));

    act(() => void result.current.refresh());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(2));
    await act(async () => {
      newerRequest.resolve(newer);
      await newerRequest.promise;
    });

    await act(async () => {
      olderRequest.reject(new Error("older failure"));
      await olderRequest.promise.catch(() => undefined);
    });
    expect(result.current.entries).toEqual(newer);
    expect(result.current.error).toBeNull();
  });

  it("keeps a newer error when an older success settles last", async () => {
    const olderRequest = deferred<InboxEntry[]>();
    const newerRequest = deferred<InboxEntry[]>();
    mocked.inbox.mockReturnValueOnce(olderRequest.promise).mockReturnValueOnce(newerRequest.promise);
    const { result } = renderHook(() => useInbox());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(1));

    act(() => void result.current.refresh());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(2));
    await act(async () => {
      newerRequest.reject(new Error("newer failure"));
      await newerRequest.promise.catch(() => undefined);
    });
    expect(result.current.error).toBe("Error: newer failure");

    await act(async () => {
      olderRequest.resolve(entries);
      await olderRequest.promise;
    });
    expect(result.current.entries).toEqual([]);
    expect(result.current.error).toBe("Error: newer failure");
  });

  it("does not install a pending request after unmount and disposes its listener", async () => {
    const request = deferred<InboxEntry[]>();
    const off = vi.fn();
    mocked.inbox.mockReturnValueOnce(request.promise);
    mocked.onCorpusChanged.mockResolvedValueOnce(off);
    const { result, unmount } = renderHook(() => useInbox());
    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mocked.onCorpusChanged).toHaveBeenCalledTimes(1));

    unmount();
    expect(off).toHaveBeenCalledTimes(1);
    await act(async () => {
      request.resolve(entries);
      await request.promise;
    });
    expect(result.current.entries).toEqual([]);
    expect(result.current.loaded).toBe(false);
  });
});

describe("CaptureBox", () => {
  it("pastes line breaks as spaces at the cursor and submits the normalized line", async () => {
    mocked.capture.mockResolvedValue({ id: "9999", at: "2026-09-13T12:00", text: "before a b c after" });
    render(<CaptureBox />);
    const input = screen.getByLabelText("Capture") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "before after" } });
    input.setSelectionRange(7, 7);
    fireEvent.paste(input, { clipboardData: { getData: () => "a\r\nb\nc " } });
    expect(input).toHaveValue("before a b c after");
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mocked.capture).toHaveBeenCalledWith("before a b c after"));
  });

  it("shows a busy retry and keeps the thought for a manual retry", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocked.capture.mockRejectedValue("corpus busy");
    render(<CaptureBox />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "keep this thought" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByText("busy, retrying…")).toBeInTheDocument();
    expect(input).toHaveValue("keep this thought");
    fireEvent.change(input, { target: { value: "keep this thought!" } });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });
    expect(mocked.capture).toHaveBeenCalledTimes(3);
    expect(screen.getByRole("status")).toHaveTextContent("Corpus busy. Press Enter to retry.");
    expect(input).toHaveValue("keep this thought!");

    mocked.capture.mockResolvedValueOnce({ id: "9999", at: "2026-09-13T12:00", text: "keep this thought!" });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByText("captured")).toBeInTheDocument();
    expect(mocked.capture).toHaveBeenLastCalledWith("keep this thought!");
    vi.useRealTimers();
  });

  it("keeps a new thought typed while capture is pending and does not dismiss it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const capture = deferred<InboxEntry>();
    const hide = vi.fn();
    mocked.capture.mockReturnValue(capture.promise);
    render(
      <CaptureBox
        onCaptured={(_, hasActiveDraft) => {
          if (!hasActiveDraft) hide();
        }}
      />,
    );
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "first thought" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mocked.capture).toHaveBeenCalledWith("first thought"));
    fireEvent.change(input, { target: { value: "second thought" } });
    await act(async () => {
      capture.resolve({ id: "9999", at: "2026-09-13T12:00", text: "first thought" });
      await capture.promise;
    });

    expect(input).toHaveValue("second thought");
    expect(screen.getByText("captured")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONFIRM_MS + 1);
    });
    expect(hide).not.toHaveBeenCalled();
    expect(input).toHaveValue("second thought");
    vi.useRealTimers();
  });

  it("keeps the current text retryable when a pending capture fails", async () => {
    const capture = deferred<InboxEntry>();
    mocked.capture.mockReturnValue(capture.promise);
    render(<CaptureBox />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "retry this thought" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mocked.capture).toHaveBeenCalledWith("retry this thought"));
    await act(async () => {
      capture.reject(new Error("offline"));
      try {
        await capture.promise;
      } catch {
        // CaptureBox renders the rejection and keeps the input retryable.
      }
    });

    expect(input).toHaveValue("retry this thought");
    expect(screen.getByRole("status")).toHaveTextContent("Error: offline");
  });

  it("does not replace newer text when an earlier capture fails", async () => {
    const capture = deferred<InboxEntry>();
    mocked.capture.mockReturnValue(capture.promise);
    render(<CaptureBox />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "failed thought" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(mocked.capture).toHaveBeenCalledWith("failed thought"));
    fireEvent.change(input, { target: { value: "new draft" } });
    await act(async () => {
      capture.reject(new Error("offline"));
      try {
        await capture.promise;
      } catch {
        // CaptureBox renders the rejection without replacing the new draft.
      }
    });

    expect(input).toHaveValue("new draft");
    expect(screen.getByRole("status")).toHaveTextContent("Error: offline");
  });
});

describe("CaptureWindow", () => {
  it("clears a previous error on reopening while keeping the draft", async () => {
    let focusChanged: (event: { payload: boolean }) => void = () => {};
    windowMock.onFocusChanged.mockImplementation((handler) => {
      focusChanged = handler;
      return Promise.resolve(() => {});
    });
    const longError = "cannot write to corpus: " + "permission denied ".repeat(30);
    mocked.capture.mockRejectedValueOnce(new Error(longError));
    render(<CaptureWindow />);
    const input = screen.getByLabelText("Capture");
    fireEvent.change(input, { target: { value: "keep this draft" } });
    fireEvent.keyDown(input, { key: "Enter" });
    const status = await screen.findByRole("status");
    await waitFor(() => expect(status).toHaveTextContent("Error: cannot write to corpus:"));
    expect(status).toHaveAttribute("title", `Error: ${longError}`);
    act(() => focusChanged({ payload: false }));
    act(() => focusChanged({ payload: true }));
    expect(status).toBeEmptyDOMElement();
    expect(input).toHaveValue("keep this draft");
  });
});

describe("App", () => {
  it("keeps graph selection, filters and viewport across a tab round-trip without reloading", async () => {
    const layoutSpy = vi.spyOn(layoutClient, "layoutGraph");
    mocked.graph.mockResolvedValue({
      nodes: [
        { id: "n0", title: "Idea 0", status: "seed", tags: ["other"], created: "", updated: "" },
        { id: "n1", title: "Idea 1", status: "seed", tags: ["focus"], created: "", updated: "" },
      ],
      edges: [],
    });
    mocked.node.mockImplementation(async (id) => ({
      node: { id, title: "Idea 1", status: "seed", created: "", updated: "" },
      body: "",
    }));
    render(<App />);
    expect(mocked.graph).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("tab", { name: "Graph" }));
    const graphCanvas = await screen.findByRole("img", { name: "Graph" });
    await waitFor(() => expect(graphCanvas.querySelectorAll("g.node")).toHaveLength(2));
    fireEvent.click(graphCanvas.querySelector('g.node[data-id="n1"]')!);
    await screen.findByRole("complementary", { name: "Node" });
    fireEvent.wheel(graphCanvas, { deltaX: 12, deltaY: 30 });
    const transform = graphCanvas.querySelector("g.scene")!.getAttribute("transform");
    fireEvent.click(screen.getByRole("button", { name: "focus1" }));
    fireEvent.change(screen.getByLabelText("Filter by title"), { target: { value: "Idea 1" } });
    await waitFor(() => expect(layoutSpy).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(graphCanvas.querySelectorAll("g.node")).toHaveLength(1));
    const layoutCount = layoutSpy.mock.calls.length;

    fireEvent.click(screen.getByRole("tab", { name: /Inbox/ }));
    fireEvent.click(screen.getByRole("tab", { name: "Graph" }));
    expect(screen.getByRole("img", { name: "Graph" })).toBe(graphCanvas);
    expect(graphCanvas.querySelector("g.scene")).toHaveAttribute("transform", transform);
    expect(screen.getByLabelText("Filter by title")).toHaveValue("Idea 1");
    expect(screen.getByRole("button", { name: "focus1" })).toHaveAttribute("aria-pressed", "true");
    expect(graphCanvas.querySelector('g.node[data-id="n1"]')).toHaveClass("node--selected");
    expect(screen.getByRole("complementary", { name: "Node" })).toBeInTheDocument();
    expect(mocked.graph).toHaveBeenCalledTimes(1);
    expect(layoutSpy).toHaveBeenCalledTimes(layoutCount);
  });

  it("shows the count in the Inbox tab", async () => {
    const inbox = deferred<InboxEntry[]>();
    mocked.inbox.mockReturnValueOnce(inbox.promise);
    render(<App />);

    await waitFor(() => expect(mocked.inbox).toHaveBeenCalledTimes(1));
    await act(async () => {
      inbox.resolve(entries);
      await inbox.promise;
    });

    expect(await screen.findByRole("tab", { name: "Inbox2" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Graph" })).toBeInTheDocument();
  });

  it("keeps the Graph tab available when Inbox fails, and reloads Inbox in place", async () => {
    mocked.inbox.mockRejectedValueOnce("no corpus at /tmp/nowhere/.nebula");
    mocked.graph.mockResolvedValue({ nodes: [], edges: [] });
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveTextContent("no corpus at /tmp/nowhere/.nebula");
    expect(await screen.findByText("/tmp/nowhere/.nebula")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Inbox" })).toContainElement(screen.getByRole("alert"));
    fireEvent.click(screen.getByRole("tab", { name: "Graph" }));
    expect(await screen.findByText(/Capture something, then promote it/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: /Inbox/ }));
    // After the fix, reload re-asks and the views come back.
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    await waitFor(() => expect(mocked.reload).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("tab", { name: /Inbox/ })).toBeInTheDocument();
    expect(screen.getByText("a1b2")).toBeInTheDocument();
  });
});

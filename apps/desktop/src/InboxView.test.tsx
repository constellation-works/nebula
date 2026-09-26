import { act, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { App } from "./App";
import { CaptureBox, CONFIRM_MS } from "./CaptureBox";
import { InboxView } from "./InboxView";
import { useInbox } from "./useInbox";
import type { InboxEntry } from "./types/InboxEntry";

vi.mock("./api");

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
  mocked.reload.mockResolvedValue(undefined);
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

describe("App", () => {
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

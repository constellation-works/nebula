import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { App } from "./App";
import { InboxView } from "./InboxView";
import { useInbox } from "./useInbox";
import type { InboxEntry } from "./types/InboxEntry";

vi.mock("./api");

const mocked = vi.mocked(api);

const entries: InboxEntry[] = [
  { id: "a1b2", at: "2026-09-13T09:00", text: "capture must stay under five seconds" },
  { id: "c3d4", at: "2026-09-13T10:30", text: "the tray count is the whole status bar" },
];

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

describe("App", () => {
  it("shows the count in the Inbox tab", async () => {
    render(<App />);
    const tab = await screen.findByRole("tab", { name: /Inbox/ });
    expect(tab).toHaveTextContent("Inbox2");
    expect(screen.getByRole("tab", { name: "Graph" })).toBeInTheDocument();
  });

  it("shows the path and a reload button when the corpus cannot be read", async () => {
    mocked.inbox.mockRejectedValueOnce("no corpus at /tmp/nowhere/.nebula");
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveTextContent("no corpus at /tmp/nowhere/.nebula");
    expect(await screen.findByText("/tmp/nowhere/.nebula")).toBeInTheDocument();
    // After the fix, reload re-asks and the views come back.
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    await waitFor(() => expect(mocked.reload).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("tab", { name: /Inbox/ })).toBeInTheDocument();
    expect(screen.getByText("a1b2")).toBeInTheDocument();
  });
});

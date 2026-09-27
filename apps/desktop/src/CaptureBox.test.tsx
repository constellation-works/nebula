import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import type { Written } from "./api";
import { CaptureBox, CONFIRM_MS } from "./CaptureBox";
import { CaptureWindow } from "./CaptureWindow";
import { LOCKED, type IpcError } from "./ipcError";
import type { InboxEntry } from "./types/InboxEntry";

vi.mock("./api");

const windowMock = vi.hoisted(() => ({ onFocusChanged: vi.fn(), hide: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));

const mocked = vi.mocked(api);

const ignored = "/corpus is ignored by the git repository that contains it; nothing can be committed";
const busyWithHolder: IpcError = {
  code: LOCKED,
  message: "another nebula writer is holding /corpus; nothing was written",
  lockHolder: { label: "neb tag foo", pid: 123 },
};

/** A capture that landed in the inbox and whose commit was refused. */
function landedUncommitted(text: string): Written<InboxEntry> {
  return {
    value: { id: "9999", at: "2026-09-13T12:00", text },
    commit: { status: "refused", error: { code: "corpus_ignored", message: ignored } },
  };
}

/** A capture that landed outside a git work tree. */
function landedWithoutRepository(text: string): Written<InboxEntry> {
  return {
    value: { id: "9999", at: "2026-09-13T12:00", text },
    commit: { status: "not_a_repository" },
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  windowMock.onFocusChanged.mockResolvedValue(() => {});
  windowMock.hide.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("CaptureBox", () => {
  it("keeps composing Enter in the draft and captures the final text once", async () => {
    mocked.capture.mockResolvedValue(landedWithoutRepository("日本語"));
    render(<CaptureBox />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "にほんご" } });
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: "Enter", isComposing: true, keyCode: 13 });
    expect(mocked.capture).not.toHaveBeenCalled();
    expect(input).toHaveValue("にほんご");

    // WebKit can dispatch compositionend before the Enter keydown. In that
    // ordering isComposing is false, and keyCode 229 identifies the IME key.
    fireEvent.compositionEnd(input, { data: "日本語" });
    fireEvent.change(input, { target: { value: "日本語" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: false, keyCode: 229 });
    expect(mocked.capture).not.toHaveBeenCalled();
    expect(input).toHaveValue("日本語");

    fireEvent.keyDown(input, { key: "Enter", isComposing: false, keyCode: 13 });
    await waitFor(() => expect(mocked.capture).toHaveBeenCalledTimes(1));
    expect(mocked.capture).toHaveBeenCalledWith("日本語");
    expect(input).toHaveValue("");
  });

  it("clears the box and warns when the capture landed but was not committed", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocked.capture.mockResolvedValue(landedUncommitted("landed once"));
    const onCaptured = vi.fn();
    render(<CaptureBox onCaptured={onCaptured} />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "landed once" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(input).toHaveValue(""));
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent(`captured (not committed: ${ignored})`);
    expect(status).toHaveClass("capture__status--warning");
    expect(onCaptured).toHaveBeenCalledTimes(1);

    // The warning outlasts the confirmation, and Enter on the cleared box
    // writes nothing a second time.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONFIRM_MS * 2);
    });
    expect(status).toHaveTextContent("not committed");
    fireEvent.keyDown(input, { key: "Enter" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONFIRM_MS);
    });
    expect(mocked.capture).toHaveBeenCalledTimes(1);
  });

  it("reports a capture that landed outside a git repository", async () => {
    let resolveCapture!: (written: Written<InboxEntry>) => void;
    mocked.capture.mockReturnValue(new Promise((resolve) => {
      resolveCapture = resolve;
    }));
    const onCaptured = vi.fn();
    render(<CaptureBox onCaptured={onCaptured} />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "saved" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(screen.getByRole("status")).toHaveTextContent("saving…");
    expect(input).toHaveValue("saved");
    expect(onCaptured).not.toHaveBeenCalled();

    await act(async () => {
      resolveCapture(landedWithoutRepository("saved"));
    });

    const status = await screen.findByText("captured (not committed: not a git repository)");
    expect(status).toHaveClass("capture__status--warning");
    expect(input).toHaveValue("");
    expect(onCaptured).toHaveBeenCalledTimes(1);
  });

  it("names a recorded lock holder after capture retries", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocked.capture.mockRejectedValue(busyWithHolder);
    render(<CaptureBox />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "keep this thought" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(mocked.capture).toHaveBeenCalledTimes(3);
    expect(screen.getByRole("status")).toHaveTextContent(
      "Corpus busy: `neb tag foo` (pid 123) is writing. Press Enter to retry.",
    );
    expect(input).toHaveValue("keep this thought");
  });
});

describe("CaptureWindow", () => {
  it("keeps the window open when Escape cancels IME composition", () => {
    render(<CaptureWindow />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "draft" } });
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: "Escape", isComposing: true, keyCode: 27 });
    expect(windowMock.hide).not.toHaveBeenCalled();
    expect(input).toHaveValue("draft");

    fireEvent.compositionEnd(input, { data: "" });
    fireEvent.keyDown(input, { key: "Escape", isComposing: false, keyCode: 27 });
    expect(windowMock.hide).toHaveBeenCalledTimes(1);
    expect(input).toHaveValue("draft");
  });

  it("stays up while a capture's commit warning shows, and Escape dismisses it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocked.capture.mockResolvedValue(landedUncommitted("saved"));
    render(<CaptureWindow />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "saved" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByText(/not committed/)).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONFIRM_MS * 2);
    });
    expect(windowMock.hide).not.toHaveBeenCalled();

    fireEvent.keyDown(input, { key: "Escape" });
    expect(windowMock.hide).toHaveBeenCalledTimes(1);
  });

  it("stays up with a not-committed note when there is no git repository", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocked.capture.mockResolvedValue(landedWithoutRepository("saved"));
    render(<CaptureWindow />);
    const input = screen.getByLabelText("Capture");

    fireEvent.change(input, { target: { value: "saved" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByText("captured (not committed: not a git repository)")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CONFIRM_MS * 2);
    });
    expect(windowMock.hide).not.toHaveBeenCalled();
  });
});

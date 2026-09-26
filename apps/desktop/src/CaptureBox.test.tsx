import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import type { Written } from "./api";
import { CaptureBox, CONFIRM_MS } from "./CaptureBox";
import { CaptureWindow } from "./CaptureWindow";
import type { InboxEntry } from "./types/InboxEntry";

vi.mock("./api");

const windowMock = vi.hoisted(() => ({ onFocusChanged: vi.fn(), hide: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));

const mocked = vi.mocked(api);

const ignored = "/corpus is ignored by the git repository that contains it; nothing can be committed";

/** A capture that landed in the inbox and whose commit was refused. */
function landedUncommitted(text: string): Written<InboxEntry> {
  return {
    value: { id: "9999", at: "2026-09-13T12:00", text },
    commit: { status: "refused", error: { code: "corpus_ignored", message: ignored } },
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
});

describe("CaptureWindow", () => {
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
});

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./api";
import { App } from "./App";

vi.mock("./api");

const mocked = vi.mocked(api);

beforeEach(() => {
  vi.resetAllMocks();
  mocked.startupWarnings.mockResolvedValue([
    "invalid /config/settings.json; using default capture shortcut `Alt+Space`",
    "could not register capture shortcut `Alt+Space` (settings: /config/settings.json): already registered",
  ]);
  mocked.inbox.mockResolvedValue([]);
  mocked.onCorpusChanged.mockResolvedValue(() => {});
  mocked.onOpenSettings.mockResolvedValue(() => {});
  mocked.captureShortcut.mockResolvedValue("Alt+Space");
  mocked.launchAtLogin.mockResolvedValue(false);
});

describe("Settings", () => {
  it("opens from the tray and applies a shortcut change without restart", async () => {
    mocked.startupWarnings.mockResolvedValue([]);
    mocked.setCaptureShortcut.mockResolvedValue("CmdOrCtrl+Shift+N");
    let openSettings: () => void = () => {};
    mocked.onOpenSettings.mockImplementation(async (handler) => {
      openSettings = handler;
      return () => {};
    });
    render(<App />);
    await waitFor(() => expect(mocked.onOpenSettings).toHaveBeenCalled());
    act(() => openSettings());
    const input = await screen.findByRole("textbox", { name: "Capture shortcut" });
    await waitFor(() => expect(input).toHaveValue("Alt+Space"));
    fireEvent.change(input, { target: { value: "CmdOrCtrl+Shift+N" } });
    fireEvent.click(screen.getByRole("button", { name: "Save shortcut" }));
    await waitFor(() => expect(mocked.setCaptureShortcut).toHaveBeenCalledWith("CmdOrCtrl+Shift+N"));
    expect(await screen.findByText("Capture shortcut saved and active.")).toBeInTheDocument();
  });

  it("shows invalid accelerator errors and leaves login state unchanged on failure", async () => {
    mocked.startupWarnings.mockResolvedValue([]);
    mocked.setCaptureShortcut.mockRejectedValue({ code: "shortcut_unparsable", message: "invalid accelerator `bogus`: unknown key" });
    mocked.setLaunchAtLogin.mockRejectedValue({ code: "launch_at_login_failed", message: "launch at login: permission denied" });
    render(<App />);
    fireEvent.click(screen.getByRole("tab", { name: "Settings" }));
    const input = await screen.findByRole("textbox", { name: "Capture shortcut" });
    await waitFor(() => expect(input).toHaveValue("Alt+Space"));
    fireEvent.change(input, { target: { value: "bogus" } });
    fireEvent.click(screen.getByRole("button", { name: "Save shortcut" }));
    expect(await screen.findByText("Shortcut not changed: invalid accelerator `bogus`: unknown key")).toBeInTheDocument();
    const checkbox = screen.getByRole("checkbox", { name: "Launch at login" });
    fireEvent.click(checkbox);
    expect(await screen.findByText("Launch at login not changed: launch at login: permission denied")).toBeInTheDocument();
    expect(checkbox).not.toBeChecked();
  });

  it("toggles launch at login through the persisted OS state", async () => {
    mocked.startupWarnings.mockResolvedValue([]);
    mocked.launchAtLogin.mockResolvedValue(true);
    mocked.setLaunchAtLogin.mockResolvedValue(false);
    render(<App />);
    fireEvent.click(screen.getByRole("tab", { name: "Settings" }));
    const checkbox = await screen.findByRole("checkbox", { name: "Launch at login" });
    await waitFor(() => expect(checkbox).toBeChecked());
    fireEvent.click(checkbox);
    await waitFor(() => expect(mocked.setLaunchAtLogin).toHaveBeenCalledWith(false));
    expect(checkbox).not.toBeChecked();
  });
});

describe("App startup warnings", () => {
  it("shows the settings fallback and shortcut registration details in the main window", async () => {
    render(<App />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Alt+Space");
    expect(alert).toHaveTextContent("/config/settings.json");
    expect(alert).toHaveTextContent("already registered");
  });
});

import { render, screen } from "@testing-library/react";
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

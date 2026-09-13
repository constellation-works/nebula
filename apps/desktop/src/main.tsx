import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { CaptureWindow } from "./CaptureWindow";
import "./styles.css";

// One bundle, two windows: which one this is decides what to draw.
function windowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>{windowLabel() === "capture" ? <CaptureWindow /> : <App />}</StrictMode>,
);

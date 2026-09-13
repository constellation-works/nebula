import { useEffect, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CaptureBox } from "./CaptureBox";

/**
 * The floating window the global shortcut opens: one input and nothing else.
 * Escape hides it; Enter captures, shows the confirmation, then hides it.
 * Hiding is the frontend's job here; showing is the shortcut's, in Rust.
 */
export function CaptureWindow() {
  const win = getCurrentWindow();
  const input = useRef<HTMLInputElement>(null);
  const hide = () => void win.hide();

  // The window is reused, so refocus the input each time it comes back.
  useEffect(() => {
    let off: (() => void) | undefined;
    void win
      .onFocusChanged(({ payload: focused }) => {
        if (focused) input.current?.focus();
      })
      .then((unlisten) => {
        off = unlisten;
      });
    return () => off?.();
  }, [win]);

  return (
    <div className="capture-window" data-tauri-drag-region>
      <CaptureBox
        ref={input}
        autoFocus
        placeholder="Capture… (Enter to save, Esc to close)"
        onCaptured={hide}
        onEscape={hide}
      />
    </div>
  );
}

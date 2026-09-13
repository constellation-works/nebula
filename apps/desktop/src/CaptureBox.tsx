import { useEffect, useImperativeHandle, useRef, useState, type KeyboardEvent, type Ref } from "react";
import * as api from "./api";
import type { InboxEntry } from "./types/InboxEntry";

/** How long the "captured" confirmation stays up. */
export const CONFIRM_MS = 1000;

interface Props {
  /** The input, for a parent that needs to refocus it. */
  ref?: Ref<HTMLInputElement>;
  /** Focus the input as soon as it mounts. */
  autoFocus?: boolean;
  placeholder?: string;
  /** After a successful capture, once the confirmation has been shown. */
  onCaptured?: (entry: InboxEntry) => void;
  onEscape?: () => void;
}

/**
 * One input. Enter captures; the box clears at once, says "captured" for a
 * second, and is ready for the next thought. The same box sits at the top of
 * the inbox and alone in the floating capture window.
 */
export function CaptureBox({ ref, autoFocus, placeholder, onCaptured, onEscape }: Props) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState<"idle" | "busy" | "captured" | "error">("idle");
  const [message, setMessage] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const input = useRef<HTMLInputElement>(null);
  useImperativeHandle(ref, () => input.current!);

  useEffect(() => () => clearTimeout(timer.current), []);

  async function submit() {
    const trimmed = text.trim();
    if (!trimmed || status === "busy") return;
    setStatus("busy");
    try {
      const entry = await api.capture(trimmed);
      setText("");
      setMessage(null);
      setStatus("captured");
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setStatus("idle");
        onCaptured?.(entry);
      }, CONFIRM_MS);
    } catch (e) {
      setStatus("error");
      setMessage(String(e));
    }
    input.current?.focus();
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      void submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onEscape?.();
    }
  }

  return (
    <div className="capture">
      <input
        ref={input}
        className="capture__input"
        type="text"
        value={text}
        placeholder={placeholder ?? "Capture a thought…"}
        aria-label="Capture"
        autoFocus={autoFocus}
        autoComplete="off"
        spellCheck
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
      />
      <span className={`capture__status capture__status--${status}`} role="status">
        {status === "captured" ? "captured" : status === "error" ? message : ""}
      </span>
    </div>
  );
}

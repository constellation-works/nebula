import { useEffect, useImperativeHandle, useRef, useState, type ClipboardEvent, type KeyboardEvent, type Ref } from "react";
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
  onCaptured?: (entry: InboxEntry, hasActiveDraft: boolean) => void;
  onEscape?: () => void;
  /** Changes when the floating window opens again. */
  resetErrorKey?: number;
}

/**
 * One input. Enter captures; after success the box clears only the submitted
 * text, says "captured" for a second, and is ready for the next thought. The
 * same box sits at the top of the inbox and alone in the floating window.
 */
export function CaptureBox({ ref, autoFocus, placeholder, onCaptured, onEscape, resetErrorKey }: Props) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState<"idle" | "busy" | "retrying" | "captured" | "error">("idle");
  const [message, setMessage] = useState<string | null>(null);
  const textRef = useRef("");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const input = useRef<HTMLInputElement>(null);
  useImperativeHandle(ref, () => input.current!);

  useEffect(() => () => clearTimeout(timer.current), []);

  useEffect(() => {
    setStatus((current) => current === "error" ? "idle" : current);
    setMessage(null);
  }, [resetErrorKey]);

  function onPaste(e: ClipboardEvent<HTMLInputElement>) {
    const pasted = e.clipboardData.getData("text");
    if (!/[\r\n]/.test(pasted)) return;
    e.preventDefault();
    const target = e.currentTarget;
    const start = target.selectionStart ?? textRef.current.length;
    const end = target.selectionEnd ?? start;
    const inserted = pasted.replace(/\r\n|\r|\n/g, " ");
    const next = textRef.current.slice(0, start) + inserted + textRef.current.slice(end);
    textRef.current = next;
    setText(next);
    queueMicrotask(() => target.setSelectionRange(start + inserted.length, start + inserted.length));
  }

  async function submit() {
    const trimmed = text.trim();
    if (!trimmed || status === "busy" || status === "retrying") return;
    const submitted = text;
    clearTimeout(timer.current);
    setMessage(null);
    setStatus("busy");
    try {
      let entry: InboxEntry;
      for (let attempt = 0; ; attempt++) {
        try {
          entry = await api.capture(trimmed);
          break;
        } catch (e) {
          if (e !== "corpus busy") throw e;
          if (attempt === 2) {
            setStatus("error");
            setMessage("Corpus busy. Press Enter to retry.");
            return;
          }
          setStatus("retrying");
          await new Promise((resolve) => setTimeout(resolve, 200));
        }
      }
      if (textRef.current === submitted) {
        textRef.current = "";
        setText("");
      }
      setMessage(null);
      setStatus("captured");
      timer.current = setTimeout(() => {
        setStatus("idle");
        onCaptured?.(entry, Boolean(textRef.current.trim()));
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
        onChange={(e) => {
          textRef.current = e.target.value;
          setText(e.target.value);
        }}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
      />
      <span className={`capture__status capture__status--${status}`} role="status" title={status === "error" ? message ?? undefined : undefined}>
        {status === "captured" ? "captured" : status === "retrying" ? "busy, retrying…" : status === "busy" ? "saving…" : status === "error" ? message : ""}
      </span>
    </div>
  );
}

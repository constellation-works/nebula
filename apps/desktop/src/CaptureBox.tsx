import { useEffect, useImperativeHandle, useRef, useState, type ClipboardEvent, type KeyboardEvent, type Ref } from "react";
import * as api from "./api";
import type { Written } from "./api";
import { errorMessage, isIpcError, lockHolderName, LOCKED } from "./ipcError";
import type { InboxEntry } from "./types/InboxEntry";

/** How long the "captured" confirmation stays up. */
export const CONFIRM_MS = 1000;

interface Props {
  /** The input, for a parent that needs to refocus it. */
  ref?: Ref<HTMLInputElement>;
  /** Focus the input as soon as it mounts. */
  autoFocus?: boolean;
  placeholder?: string;
  /**
   * After a capture landed: once the confirmation has been shown, or at once
   * when its commit was refused or could not be committed because there is
   * no git work tree, and the warning stays up.
   */
  onCaptured?: (written: Written<InboxEntry>, hasActiveDraft: boolean) => void;
  onEscape?: () => void;
  /** Changes when the floating window opens again. */
  resetErrorKey?: number;
}

/**
 * One input. Enter captures; after success the box clears only the submitted
 * text, says "captured" for a second, and is ready for the next thought. A
 * capture whose commit was refused or had no repository landed all the same:
 * the text is cleared too, so Enter cannot write it twice, and the status
 * keeps a warning saying it was not committed. The same box sits at the top
 * of the inbox and alone in the floating window.
 */
export function CaptureBox({ ref, autoFocus, placeholder, onCaptured, onEscape, resetErrorKey }: Props) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState<"idle" | "busy" | "retrying" | "captured" | "warning" | "error">("idle");
  const [message, setMessage] = useState<string | null>(null);
  const textRef = useRef("");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const input = useRef<HTMLInputElement>(null);
  useImperativeHandle(ref, () => input.current!);

  useEffect(() => () => clearTimeout(timer.current), []);

  useEffect(() => {
    setStatus((current) => current === "error" || current === "warning" ? "idle" : current);
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
      let written: Written<InboxEntry>;
      for (let attempt = 0; ; attempt++) {
        try {
          written = await api.capture(trimmed);
          break;
        } catch (e) {
          if (!isIpcError(e) || e.code !== LOCKED) throw e;
          if (attempt === 2) {
            setStatus("error");
            const holder = lockHolderName(e);
            setMessage(
              holder
                ? `Corpus busy: ${holder} is writing. Press Enter to retry.`
                : "Corpus busy. Press Enter to retry.",
            );
            return;
          }
          setStatus("retrying");
          await new Promise((resolve) => setTimeout(resolve, 200));
        }
      }
      // The line is in the inbox whatever the commit did, so the text goes.
      if (textRef.current === submitted) {
        textRef.current = "";
        setText("");
      }
      if (written.commit.status === "refused" || written.commit.status === "not_a_repository") {
        setStatus("warning");
        const reason = written.commit.status === "refused" ? errorMessage(written.commit.error) : "not a git repository";
        setMessage(`captured (not committed: ${reason})`);
        onCaptured?.(written, Boolean(textRef.current.trim()));
      } else {
        setMessage(null);
        setStatus("captured");
        timer.current = setTimeout(() => {
          setStatus("idle");
          onCaptured?.(written, Boolean(textRef.current.trim()));
        }, CONFIRM_MS);
      }
    } catch (e) {
      setStatus("error");
      setMessage(errorMessage(e));
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
      <span className={`capture__status capture__status--${status}`} role="status" title={status === "error" || status === "warning" ? message ?? undefined : undefined}>
        {status === "captured" ? "captured" : status === "retrying" ? "busy, retrying…" : status === "busy" ? "saving…" : status === "error" || status === "warning" ? message : ""}
      </span>
    </div>
  );
}

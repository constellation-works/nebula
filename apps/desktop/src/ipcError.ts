// What a failed command rejects with, and the one way to read it. Kept out of
// `api.ts` so a test that mocks every command there still gets the real
// guard and message helper.

/**
 * Every `#[tauri::command]` rejects with this: `IpcError` in
 * `src-tauri/src/error.rs`. Branch on `code`, show `message`; never match
 * the message text.
 */
export interface IpcError {
  /** Stable machine name: nebula-core's code (as `neb --json` prints it) or a desktop one. */
  code: string;
  /** The error's own message, which names what went wrong. */
  message: string;
}

/** Another writer held the corpus lock past the desktop's short wait; nothing was written. */
export const LOCKED = "locked";

/** Whether a rejection is a command's `IpcError`. */
export function isIpcError(reason: unknown): reason is IpcError {
  return (
    typeof reason === "object" &&
    reason !== null &&
    typeof (reason as Record<string, unknown>).code === "string" &&
    typeof (reason as Record<string, unknown>).message === "string"
  );
}

/**
 * The text to show for any rejection: a command's message, or whatever else
 * was thrown as text. An object never renders as `[object Object]`.
 */
export function errorMessage(reason: unknown): string {
  return isIpcError(reason) ? reason.message : String(reason);
}

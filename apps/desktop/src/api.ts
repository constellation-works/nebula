// The IPC surface, one function per `#[tauri::command]` in
// `src-tauri/src/commands.rs`. Every payload type comes from `./types`, which
// is generated from nebula-core, except the desktop's own `Written` wrapper
// declared below. A failed command rejects with an `IpcError` (`./ipcError`):
// branch on its `code`, show it with `errorMessage`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl as pluginOpenUrl } from "@tauri-apps/plugin-opener";
import type { IpcError } from "./ipcError";
import type { Committed } from "./types/Committed";
import type { Created } from "./types/Created";
import type { GraphExport } from "./types/GraphExport";
import type { Inbox } from "./types/Inbox";
import type { InboxEntry } from "./types/InboxEntry";
import type { NodeView } from "./types/NodeView";

/**
 * What the commit after a write did: `CommitReport` in
 * `src-tauri/src/session.rs`, whose JSON `tests/commands.rs` pins against
 * these lines. `refused` means the write landed and was not committed: a
 * warning, never a reason to write again.
 */
export type CommitReport =
  | { status: "committed"; commit: Committed }
  | { status: "disabled" }
  | { status: "not_a_repository" }
  | { status: "nothing_to_commit" }
  | { status: "refused"; error: IpcError };

/**
 * A write that landed: what it produced, and what the commit after it did
 * (`Written<T>` in `src-tauri/src/session.rs`). A write command rejects only
 * when nothing was written.
 */
export interface Written<T> {
  value: T;
  commit: CommitReport;
}

/** Append one line to this month's inbox file. */
export const capture = (text: string): Promise<Written<InboxEntry>> =>
  invoke<Written<InboxEntry>>("capture", { text });

/** Every unsettled capture, oldest first. */
export const inbox = (): Promise<Inbox> => invoke<Inbox>("inbox");

/** Settle one entry as dropped through nebula-core. */
export const dropEntry = (entry: string): Promise<Written<InboxEntry>> =>
  invoke<Written<InboxEntry>>("drop_entry", { entry });

/** Promote captured text to an unlinked root node through nebula-core. */
export const promoteRoot = (entry: string): Promise<Written<Created>> =>
  invoke<Written<Created>>("promote_root", { entry });

/** The whole corpus as nodes and edges. */
export const graph = (): Promise<GraphExport> => invoke<GraphExport>("graph");

/** Matching graph IDs from one corpus read, including markdown bodies. */
export const graphSearch = (query: string): Promise<string[]> => invoke<string[]>("graph_search", { query });

/** The capture shortcut currently used by the app. */
export const captureShortcut = (): Promise<string> => invoke<string>("capture_shortcut");

/** Register a replacement immediately and save it for the next launch. */
export const setCaptureShortcut = (shortcut: string): Promise<string> =>
  invoke<string>("set_capture_shortcut", { shortcut });

/** Read and change the OS login registration. */
export const launchAtLogin = (): Promise<boolean> => invoke<boolean>("launch_at_login");
export const setLaunchAtLogin = (enabled: boolean): Promise<boolean> =>
  invoke<boolean>("set_launch_at_login", { enabled });

/** Open Settings when the tray menu requests it. */
export const onOpenSettings = (handler: () => void): Promise<UnlistenFn> =>
  listen("show-settings", handler);

/** One node in full: frontmatter and trimmed markdown body. */
export const node = (id: string): Promise<NodeView> => invoke<NodeView>("node", { id });

/** Hand the node's file to the OS default handler. */
export const openInEditor = (id: string): Promise<void> => invoke<void>("open_in_editor", { id });

/**
 * Where the corpus was looked for, found or not; null when the root could not
 * be resolved, which the corpus commands' error and the startup warnings say.
 */
export const corpusPath = (): Promise<string | null> => invoke<string | null>("corpus_path");

/** Settings and shortcut failures captured during app startup. */
export const startupWarnings = (): Promise<string[]> => invoke<string[]>("startup_warnings");

/** Try the corpus again after the path has been fixed. */
export const reload = (): Promise<void> => invoke<void>("reload");

/**
 * Fires after `nodes/` or `inbox/` changed on disk, whoever changed them. The
 * payload is empty on purpose: refetch what is shown.
 */
export const onCorpusChanged = (handler: () => void): Promise<UnlistenFn> =>
  listen("corpus-changed", handler);

/**
 * Open a URL in the OS default handler, via the opener plugin. The webview
 * must never navigate itself, so every external link in the UI routes here.
 */
export const openUrl = (url: string): Promise<void> => pluginOpenUrl(url);

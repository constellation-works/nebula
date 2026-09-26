// The IPC surface, one function per `#[tauri::command]` in
// `src-tauri/src/commands.rs`. Every payload type comes from `./types`, which
// is generated from nebula-core: nothing here restates a shape.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl as pluginOpenUrl } from "@tauri-apps/plugin-opener";
import type { Created } from "./types/Created";
import type { GraphExport } from "./types/GraphExport";
import type { Inbox } from "./types/Inbox";
import type { InboxEntry } from "./types/InboxEntry";
import type { NodeView } from "./types/NodeView";

/** Append one line to this month's inbox file. */
export const capture = (text: string): Promise<InboxEntry> =>
  invoke<InboxEntry>("capture", { text });

/** Every unsettled capture, oldest first. */
export const inbox = (): Promise<Inbox> => invoke<Inbox>("inbox");

/** Settle one entry as dropped through nebula-core. */
export const dropEntry = (entry: string): Promise<InboxEntry> =>
  invoke<InboxEntry>("drop_entry", { entry });

/** Promote captured text to an unlinked root node through nebula-core. */
export const promoteRoot = (entry: string): Promise<Created> =>
  invoke<Created>("promote_root", { entry });

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

/** Where the corpus was looked for, found or not. */
export const corpusPath = (): Promise<string> => invoke<string>("corpus_path");

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

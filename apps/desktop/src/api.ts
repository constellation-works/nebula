// The IPC surface, one function per `#[tauri::command]` in
// `src-tauri/src/commands.rs`. Every payload type comes from `./types`, which
// is generated from nebula-core: nothing here restates a shape.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl as pluginOpenUrl } from "@tauri-apps/plugin-opener";
import type { GraphExport } from "./types/GraphExport";
import type { Inbox } from "./types/Inbox";
import type { InboxEntry } from "./types/InboxEntry";
import type { NodeView } from "./types/NodeView";

/** Append one line to this month's inbox file. */
export const capture = (text: string): Promise<InboxEntry> =>
  invoke<InboxEntry>("capture", { text });

/** Every unsettled capture, oldest first. */
export const inbox = (): Promise<Inbox> => invoke<Inbox>("inbox");

/** The whole corpus as nodes and edges. */
export const graph = (): Promise<GraphExport> => invoke<GraphExport>("graph");

/** One node in full: frontmatter and trimmed markdown body. */
export const node = (id: string): Promise<NodeView> => invoke<NodeView>("node", { id });

/** Hand the node's file to the OS default handler. */
export const openInEditor = (id: string): Promise<void> => invoke<void>("open_in_editor", { id });

/** Where the corpus was looked for, found or not. */
export const corpusPath = (): Promise<string> => invoke<string>("corpus_path");

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

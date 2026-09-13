import { useCallback, useEffect, useState } from "react";
import * as api from "./api";
import type { GraphExport } from "./types/GraphExport";

/** The one store the graph view keys on: the export, and how to ask again. */
export interface GraphState {
  /** Null until the first answer. A new object on every refetch. */
  graph: GraphExport | null;
  /** The backend's message when the corpus cannot be read; null when it can. */
  error: string | null;
  loaded: boolean;
  refresh: () => Promise<void>;
}

/**
 * One `graph()` call, refetched on `corpus-changed` so a `neb link` in a
 * terminal redraws the view. Selection and viewport are not here on purpose:
 * they belong to the view and must survive a refetch.
 */
export function useGraph(): GraphState {
  const [graph, setGraph] = useState<GraphExport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setGraph(await api.graph());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | undefined;
    void refresh();
    api
      .onCorpusChanged(() => void refresh())
      .then((off) => {
        if (live) unlisten = off;
        else off();
      })
      .catch(() => {
        // Outside Tauri there is no event bus; the graph still loads once.
      });
    return () => {
      live = false;
      unlisten?.();
    };
  }, [refresh]);

  return { graph, error, loaded, refresh };
}

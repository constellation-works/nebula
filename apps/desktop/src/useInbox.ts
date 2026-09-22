import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "./api";
import type { InboxEntry } from "./types/InboxEntry";

/** What the inbox looks like right now, and how to ask again. */
export interface InboxState {
  entries: InboxEntry[];
  /** The backend's message when the corpus cannot be read; null when it can. */
  error: string | null;
  loaded: boolean;
  refresh: () => Promise<void>;
}

/**
 * The unsettled inbox, refetched on `corpus-changed` so a `neb capture` in a
 * terminal shows up here without a click.
 */
export function useInbox(): InboxState {
  const [entries, setEntries] = useState<InboxEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const mounted = useRef(false);
  const latestRequest = useRef(0);

  const refresh = useCallback(async () => {
    if (!mounted.current) return;
    const request = ++latestRequest.current;
    try {
      const next = await api.inbox();
      if (!mounted.current || request !== latestRequest.current) return;
      setEntries(next);
      setError(null);
    } catch (e) {
      if (!mounted.current || request !== latestRequest.current) return;
      setError(String(e));
    } finally {
      if (mounted.current && request === latestRequest.current) setLoaded(true);
    }
  }, []);

  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | undefined;
    mounted.current = true;
    void refresh();
    api
      .onCorpusChanged(() => void refresh())
      .then((off) => {
        if (live) unlisten = off;
        else off();
      })
      .catch(() => {
        // Outside Tauri there is no event bus; the list still loads once.
      });
    return () => {
      live = false;
      mounted.current = false;
      latestRequest.current += 1;
      unlisten?.();
    };
  }, [refresh]);

  return { entries, error, loaded, refresh };
}

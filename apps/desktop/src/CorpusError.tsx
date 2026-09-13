import { useEffect, useState } from "react";
import * as api from "./api";

interface Props {
  /** The backend's message, which names what went wrong. */
  message: string;
  /** Called after a reload attempt so the views refetch. */
  onReloaded: () => void;
}

/**
 * Shown instead of the views when the corpus cannot be read. Says where it
 * looked, so a wrong `NEBULA_ROOT` is obvious, and offers to look again.
 */
export function CorpusError({ message, onReloaded }: Props) {
  const [path, setPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void api.corpusPath().then(setPath, () => setPath(null));
  }, []);

  async function reload() {
    setBusy(true);
    try {
      await api.reload();
    } catch {
      // The message on screen is refreshed by the refetch below.
    } finally {
      setBusy(false);
      onReloaded();
    }
  }

  return (
    <section className="error" role="alert">
      <h1 className="error__title">Cannot read the corpus</h1>
      <p className="error__message">{message}</p>
      {path && (
        <p className="error__path">
          Looked in <code>{path}</code>
        </p>
      )}
      <p className="error__hint">
        Set <code>NEBULA_ROOT</code> to your corpus, or run <code>neb init</code> there, then
        reload.
      </p>
      <button className="error__reload" type="button" onClick={() => void reload()} disabled={busy}>
        Reload
      </button>
    </section>
  );
}

import { useState } from "react";
import * as api from "./api";
import { CaptureBox } from "./CaptureBox";
import { CorpusError } from "./CorpusError";
import { formatAge, isStale } from "./age";
import type { InboxEntry } from "./types/InboxEntry";
import type { InboxState } from "./useInbox";

interface Props {
  inbox: InboxState;
}

/**
 * The capture box, then every unsettled entry oldest first: id, age, text,
 * and the two decisions the desktop can make without further details.
 */
export function InboxView({ inbox }: Props) {
  const { entries, error, loaded, refresh } = inbox;
  return (
    <section className="inbox" aria-label="Inbox">
      {error !== null ? (
        <CorpusError message={error} onReloaded={() => void refresh()} />
      ) : (
        <>
          <CaptureBox onCaptured={() => void refresh()} />
          {loaded && entries.length === 0 ? (
            <p className="inbox__empty">Nothing waiting.</p>
          ) : (
            <ul className="inbox__list">
              {entries.map((entry) => (
                <InboxItem key={`${entry.at}-${entry.id}`} entry={entry} refresh={refresh} />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

function InboxItem({ entry, refresh }: { entry: InboxEntry; refresh: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function settle(action: (id: string) => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await action(entry.id);
      await refresh();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <li className="entry">
      <code className="entry__id">{entry.id}</code>
      <time className="entry__age" dateTime={entry.at} title={entry.at}>
        {formatAge(entry.at)}
      </time>
      <span className="entry__text">{entry.text}</span>
      {isStale(entry.at) && <span className="entry__stale">stale</span>}
      <div className="entry__actions">
        <button type="button" disabled={busy} onClick={() => void settle(api.promoteRoot)}>
          Promote as root
        </button>
        <button type="button" disabled={busy} onClick={() => void settle(api.dropEntry)}>
          Drop
        </button>
      </div>
      {error && <p className="entry__error" role="alert">{error}</p>}
    </li>
  );
}

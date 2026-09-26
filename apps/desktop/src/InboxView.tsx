import { useState } from "react";
import * as api from "./api";
import type { Written } from "./api";
import { CaptureBox } from "./CaptureBox";
import { CorpusError } from "./CorpusError";
import { formatAge, isStale } from "./age";
import { errorMessage, isIpcError, lockHolderName, LOCKED } from "./ipcError";
import type { InboxEntry } from "./types/InboxEntry";
import type { InboxState } from "./useInbox";

interface Props {
  inbox: InboxState;
}

/**
 * The capture box, then every unsettled entry oldest first: id, age, text,
 * and the two decisions the desktop can make without further details. A
 * settle whose commit was refused or had no repository still settled: the
 * entry leaves the list, and a note above it says the change is not committed.
 */
export function InboxView({ inbox }: Props) {
  const { entries, error, loaded, refresh } = inbox;
  const [warning, setWarning] = useState<string | null>(null);
  return (
    <section className="inbox" aria-label="Inbox">
      {error !== null ? (
        <CorpusError message={error} onReloaded={() => void refresh()} />
      ) : (
        <>
          <CaptureBox onCaptured={() => void refresh()} />
          {warning && <p className="inbox__warning" role="status">{warning}</p>}
          {loaded && entries.length === 0 ? (
            <p className="inbox__empty">Nothing waiting.</p>
          ) : (
            <ul className="inbox__list">
              {entries.map((entry) => (
                <InboxItem key={`${entry.at}-${entry.id}`} entry={entry} refresh={refresh} onWarning={setWarning} />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

interface ItemProps {
  entry: InboxEntry;
  refresh: () => Promise<void>;
  /** Say, above the list, that a settle landed but was not committed. */
  onWarning: (warning: string | null) => void;
}

function InboxItem({ entry, refresh, onWarning }: ItemProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function settle(action: (id: string) => Promise<Written<unknown>>, done: string) {
    if (busy) return;
    setBusy(true);
    setError(null);
    onWarning(null);
    try {
      const written = await action(entry.id);
      // Settled either way; only the commit is missing, so no retry.
      if (written.commit.status === "refused" || written.commit.status === "not_a_repository") {
        const reason = written.commit.status === "refused" ? errorMessage(written.commit.error) : "not a git repository";
        onWarning(`${done} ${entry.id} (not committed: ${reason})`);
      }
      await refresh();
    } catch (cause) {
      const locked = isIpcError(cause) && cause.code === LOCKED;
      if (locked) {
        const holder = lockHolderName(cause);
        setError(holder ? `Corpus busy: ${holder} is writing. Try again.` : "Corpus busy; nothing was changed. Try again.");
      } else {
        setError(errorMessage(cause));
      }
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
        <button type="button" disabled={busy} onClick={() => void settle(api.promoteRoot, "promoted")}>
          Promote as root
        </button>
        <button type="button" disabled={busy} onClick={() => void settle(api.dropEntry, "dropped")}>
          Drop
        </button>
      </div>
      {error && <p className="entry__error" role="alert">{error}</p>}
    </li>
  );
}

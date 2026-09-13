import { CaptureBox } from "./CaptureBox";
import { formatAge } from "./age";
import type { InboxState } from "./useInbox";

interface Props {
  inbox: InboxState;
}

/**
 * The capture box, then every unsettled entry oldest first: id, age, text.
 * Read-only past the capture box; triage happens in a session with the agent.
 */
export function InboxView({ inbox }: Props) {
  const { entries, loaded, refresh } = inbox;
  return (
    <section className="inbox" aria-label="Inbox">
      <CaptureBox onCaptured={() => void refresh()} />
      {loaded && entries.length === 0 ? (
        <p className="inbox__empty">Nothing waiting.</p>
      ) : (
        <ul className="inbox__list">
          {entries.map((e) => (
            <li key={`${e.at}-${e.id}`} className="entry">
              <code className="entry__id">{e.id}</code>
              <time className="entry__age" dateTime={e.at} title={e.at}>
                {formatAge(e.at)}
              </time>
              <span className="entry__text">{e.text}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

import { useEffect, useState } from "react";
import { startupWarnings as getStartupWarnings } from "./api";
import { GraphView } from "./GraphView";
import { InboxView } from "./InboxView";
import { useInbox } from "./useInbox";

type Tab = "inbox" | "graph";

/** The main window: two tabs over one corpus. */
export function App() {
  const [tab, setTab] = useState<Tab>("inbox");
  const [graphVisited, setGraphVisited] = useState(false);
  const [startupWarnings, setStartupWarnings] = useState<string[]>([]);
  const inbox = useInbox();

  useEffect(() => {
    let active = true;
    void getStartupWarnings()
      .then((warnings) => {
        if (active) setStartupWarnings(warnings);
      })
      .catch((error: unknown) => {
        if (active) setStartupWarnings([`Could not load startup warnings: ${String(error)}`]);
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <div className="app">
      {startupWarnings.length > 0 && (
        <section className="startup-warning" role="alert" aria-labelledby="startup-warning-title">
          <h2 id="startup-warning-title">Startup warning</h2>
          <ul>
            {startupWarnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </section>
      )}
      <nav className="tabs" role="tablist">
        <button
          role="tab"
          type="button"
          className="tab"
          aria-selected={tab === "inbox"}
          onClick={() => setTab("inbox")}
        >
          Inbox
          {inbox.entries.length > 0 && <span className="tab__count">{inbox.entries.length}</span>}
        </button>
        <button
          role="tab"
          type="button"
          className="tab"
          aria-selected={tab === "graph"}
          onClick={() => {
            setGraphVisited(true);
            setTab("graph");
          }}
        >
          Graph
        </button>
      </nav>
      <main className="view" hidden={tab !== "inbox"}>
        <InboxView inbox={inbox} />
      </main>
      {graphVisited && (
        <main className="view" hidden={tab !== "graph"}>
          <GraphView active={tab === "graph"} />
        </main>
      )}
    </div>
  );
}

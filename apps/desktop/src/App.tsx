import { useState } from "react";
import { GraphView } from "./GraphView";
import { InboxView } from "./InboxView";
import { useInbox } from "./useInbox";

type Tab = "inbox" | "graph";

/** The main window: two tabs over one corpus. */
export function App() {
  const [tab, setTab] = useState<Tab>("inbox");
  const inbox = useInbox();

  return (
    <div className="app">
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
          onClick={() => setTab("graph")}
        >
          Graph
        </button>
      </nav>
      <main className="view">{tab === "inbox" ? <InboxView inbox={inbox} /> : <GraphView />}</main>
    </div>
  );
}

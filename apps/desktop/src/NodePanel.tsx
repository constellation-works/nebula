import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import * as api from "./api";
import { isGenealogy } from "./layout";
import type { Edge } from "./types/Edge";
import type { NodeSummary } from "./types/NodeSummary";
import type { NodeView } from "./types/NodeView";

interface Props {
  id: string;
  /** Titles for the edge lists; the node view only carries ids. */
  nodes: readonly NodeSummary[];
  /** Any new value refetches the node, so a body edited on disk shows up. */
  revision: unknown;
  width: number;
  onResize: (width: number) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
}

export const PANEL_MIN = 280;
export const PANEL_MAX = 720;

/** Only these leave the app; a repo path or almanac wikilink is shown as text. */
const isExternal = (uri: string): boolean => /^(https?:|mailto:)/i.test(uri);

/** An `<a>` that hands the URL to the OS instead of navigating the webview. */
function ExternalLink({ href, children }: { href?: string; children?: ReactNode }) {
  if (href === undefined || !isExternal(href)) return <code>{children ?? href}</code>;
  return (
    <a
      href={href}
      rel="noreferrer"
      onClick={(e) => {
        e.preventDefault();
        void api.openUrl(href);
      }}
    >
      {children}
    </a>
  );
}

function EdgeList({
  heading,
  edges,
  titles,
  onSelect,
}: {
  heading: string;
  edges: Edge[];
  titles: Map<string, string>;
  onSelect: (id: string) => void;
}) {
  if (edges.length === 0) return null;
  return (
    <section className="panel__section">
      <h3 className="panel__heading">{heading}</h3>
      <ul className="panel__edges">
        {edges.map((e) => (
          <li key={`${e.type}:${e.to}`}>
            <span className="panel__edge-type">{e.type}</span>
            <button type="button" className="panel__link" onClick={() => onSelect(e.to)}>
              {titles.get(e.to) ?? e.to}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * The right-hand panel: one node in full, read-only. Edits happen in a
 * session with the agent or in the editor, which is one double-click away.
 */
export function NodePanel({ id, nodes, revision, width, onResize, onSelect, onClose }: Props) {
  const [view, setView] = useState<NodeView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const resize = useRef<{ x: number; w: number } | null>(null);

  useEffect(() => {
    let live = true;
    api.node(id).then(
      (v) => {
        if (!live) return;
        setView(v);
        setError(null);
      },
      (e: unknown) => {
        if (live) setError(String(e));
      },
    );
    return () => {
      live = false;
    };
  }, [id, revision]);

  // The handle on the left edge: drag to resize, released anywhere.
  useEffect(() => {
    const move = (e: MouseEvent) => {
      const r = resize.current;
      if (r === null) return;
      onResize(Math.min(PANEL_MAX, Math.max(PANEL_MIN, r.w + (r.x - e.clientX))));
    };
    const up = () => {
      resize.current = null;
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [onResize]);

  function startResize(e: ReactMouseEvent) {
    e.preventDefault();
    resize.current = { x: e.clientX, w: width };
  }

  const titles = new Map(nodes.map((n) => [n.id, n.title]));
  const node = view?.node;
  const edges = node?.edges ?? [];
  const references = node?.references ?? [];

  return (
    <aside className="panel" aria-label="Node" style={{ width }}>
      <div
        className="panel__handle"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize panel"
        onMouseDown={startResize}
      />
      <div className="panel__body">
        <header className="panel__top">
          <code className="panel__id">{id}</code>
          <span className="panel__spacer" />
          <button type="button" className="panel__action" onClick={() => void api.openInEditor(id)}>
            Open file
          </button>
          <button type="button" className="panel__action" onClick={onClose} aria-label="Close panel">
            ×
          </button>
        </header>
        {error !== null && (
          <p className="panel__error" role="alert">
            {error}
          </p>
        )}
        {node !== undefined && view !== null && (
          <>
            <h2 className="panel__title">{node.title}</h2>
            <div className="panel__meta">
              <span className={`badge badge--${node.status}`}>{node.status}</span>
              <span className="panel__dates">
                {node.created} · updated {node.updated}
              </span>
            </div>
            {(node.tags ?? []).length > 0 && (
              <ul className="panel__tags" aria-label="Tags">
                {node.tags!.map((t) => (
                  <li key={t} className="tag">
                    {t}
                  </li>
                ))}
              </ul>
            )}
            {node.kill && (
              <section className="panel__section panel__kill">
                <h3 className="panel__heading">Kill condition</h3>
                <p>{node.kill}</p>
              </section>
            )}
            {node.closed && (
              <section className="panel__section panel__closed">
                <h3 className="panel__heading">Closed {node.closed.at}</h3>
                <p>{node.closed.why}</p>
              </section>
            )}
            {view.body.trim() !== "" && (
              <section className="panel__section markdown">
                <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ a: ExternalLink }}>
                  {view.body}
                </ReactMarkdown>
              </section>
            )}
            <EdgeList heading="Genealogy" edges={edges.filter(isGenealogy)} titles={titles} onSelect={onSelect} />
            <EdgeList
              heading="Contradicts"
              edges={edges.filter((e) => !isGenealogy(e))}
              titles={titles}
              onSelect={onSelect}
            />
            {references.length > 0 && (
              <section className="panel__section">
                <h3 className="panel__heading">References</h3>
                <table className="panel__refs">
                  <thead>
                    <tr>
                      <th>kind</th>
                      <th>title</th>
                      <th>note</th>
                      <th>added</th>
                    </tr>
                  </thead>
                  <tbody>
                    {references.map((r) => (
                      <tr key={r.id}>
                        <td>{r.kind}</td>
                        <td>
                          <ExternalLink href={r.uri}>{r.title ?? r.uri}</ExternalLink>
                        </td>
                        <td>{r.note ?? ""}</td>
                        <td>{r.added}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </section>
            )}
          </>
        )}
      </div>
    </aside>
  );
}

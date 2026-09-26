import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import { GraphCanvas } from "./GraphCanvas";
import { filterGraph, fromElkLayout, lineage, tagCounts, toElkGraph, type Layout } from "./layout";
import { layoutGraph } from "./layoutClient";
import { NodePanel } from "./NodePanel";
import { useGraph } from "./useGraph";

const EMPTY: Layout = { width: 0, height: 0, nodes: [], edges: [] };
const PANEL_DEFAULT = 380;
const FILTER_DEBOUNCE_MS = 250;

/**
 * The whole corpus as a layered DAG, with a toolbar that narrows it and a
 * panel that shows one node. One `graph()` export feeds everything; the
 * selection and the viewport live here and outlast a refetch.
 */
export function GraphView({ active = true }: { active?: boolean }) {
  const { graph, error, loaded, refresh } = useGraph();
  const [query, setQuery] = useState("");
  const [appliedQuery, setAppliedQuery] = useState({ query: "", revision: 0 });
  const [tags, setTags] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [panelWidth, setPanelWidth] = useState(PANEL_DEFAULT);
  const [layout, setLayout] = useState<Layout>(EMPTY);
  const [laying, setLaying] = useState(false);
  const [layoutError, setLayoutError] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);
  const [timing, setTiming] = useState<{ layout: number; paint: number } | null>(null);
  const [fitRequest, setFitRequest] = useState(0);
  const started = useRef(0);
  const layoutVersion = useRef(0);

  useEffect(() => {
    if (query === appliedQuery.query && layoutVersion.current === appliedQuery.revision) return;
    const timer = setTimeout(() => {
      setAppliedQuery({ query, revision: layoutVersion.current });
    }, FILTER_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query, appliedQuery]);

  const counts = useMemo(() => (graph === null ? [] : tagCounts(graph.nodes)), [graph]);
  const filtered = useMemo(
    () => (graph === null ? null : filterGraph(graph, { query: appliedQuery.query, tags })),
    [graph, appliedQuery, tags],
  );

  // Every change to what is drawn re-runs layout; the previous drawing stays
  // up until the new one lands, so the canvas never flashes empty.
  useEffect(() => {
    if (filtered === null) return;
    if (filtered.nodes.length === 0) {
      setLayout(EMPTY);
      setLaying(false);
      return;
    }
    let live = true;
    const version = layoutVersion.current;
    started.current = performance.now();
    setLaying(true);
    layoutGraph(toElkGraph(filtered)).then(
      (laid) => {
        if (!live || version !== layoutVersion.current) return;
        const ms = performance.now() - started.current;
        setLayout(fromElkLayout(laid, filtered));
        setTiming({ layout: ms, paint: 0 });
        setLayoutError(null);
        setLaying(false);
      },
      (e: unknown) => {
        if (!live || version !== layoutVersion.current) return;
        setLayoutError(String(e));
        setLaying(false);
      },
    );
    return () => {
      live = false;
    };
  }, [filtered]);

  // Paint time: from the layout request to the frame after the new drawing
  // committed. Shown in the toolbar so the number is checkable in the app.
  useEffect(() => {
    if (layout === EMPTY || typeof requestAnimationFrame !== "function") return;
    const raf = requestAnimationFrame(() => {
      const paint = performance.now() - started.current;
      setTiming((t) => (t === null ? t : { ...t, paint }));
    });
    return () => cancelAnimationFrame(raf);
  }, [layout]);

  // A node that left the corpus, or the filter, cannot stay selected.
  useEffect(() => {
    if (selected !== null && filtered !== null && !filtered.nodes.some((n) => n.id === selected)) {
      setSelected(null);
    }
  }, [filtered, selected]);

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSelected(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

  const lin = useMemo(
    () => (selected === null || filtered === null ? null : lineage(filtered.edges, selected)),
    [filtered, selected],
  );

  const open = useCallback((id: string) => {
    setOpenError(null);
    void api.openInEditor(id).catch((e: unknown) => setOpenError(`Could not open file: ${String(e)}`));
  }, []);
  const reload = async () => {
    setReloading(true);
    try {
      await api.reload();
    } catch {
      // The refetch below reports the current graph error.
    } finally {
      await refresh();
      setReloading(false);
    }
  };
  const graphError = (message: string) => (
    <div className="graph__error" role="alert">
      <p>{message}</p>
      <p>
        Run <code>neb check</code> to find problems in the corpus, then reload.
      </p>
      <button type="button" onClick={() => void reload()} disabled={reloading}>
        Reload
      </button>
    </div>
  );
  const toggleTag = (t: string) => {
    const revision = ++layoutVersion.current;
    setAppliedQuery((cur) => ({ ...cur, revision }));
    setTags((cur) => (cur.includes(t) ? cur.filter((x) => x !== t) : [...cur, t]));
  };
  const clearQuery = () => {
    const revision = ++layoutVersion.current;
    setQuery("");
    setAppliedQuery({ query: "", revision });
  };

  if (error !== null) {
    return (
      <section className="graph" aria-label="Graph">
        {graphError(error)}
      </section>
    );
  }
  if (!loaded || graph === null) {
    return (
      <section className="graph" aria-label="Graph">
        <p className="graph__empty" role="status">Loading graph…</p>
      </section>
    );
  }
  if (graph.nodes.length === 0) {
    return (
      <section className="graph" aria-label="Graph">
        <p className="graph__empty">
          Nothing in the graph yet. Capture something, then promote it with the agent.
        </p>
      </section>
    );
  }

  const filtering = query.trim() !== "" || tags.length > 0;
  const shown = filtered?.nodes.length ?? 0;

  return (
    <section className="graph" aria-label="Graph">
      {openError !== null && (
        <div className="graph__open-error" role="alert">
          <span>{openError}</span>
          <button type="button" onClick={() => setOpenError(null)} aria-label="Dismiss open error">
            Dismiss
          </button>
        </div>
      )}
      <div className="toolbar" role="toolbar" aria-label="Graph filters">
        <input
          className="toolbar__search"
          type="search"
          placeholder="Filter by title"
          aria-label="Filter by title"
          value={query}
          onChange={(e) => {
            layoutVersion.current += 1;
            setQuery(e.target.value);
          }}
          onKeyDown={(e) => {
            // Escape in the box clears the box, and only the box.
            if (e.key === "Escape" && query !== "") {
              e.stopPropagation();
              clearQuery();
            }
          }}
        />
        {counts.length > 0 && (
          <ul className="toolbar__tags" aria-label="Filter by tag">
            {counts.map(({ tag, count }) => (
              <li key={tag}>
                <button
                  type="button"
                  className="tagbtn"
                  aria-pressed={tags.includes(tag)}
                  onClick={() => toggleTag(tag)}
                >
                  {tag}
                  <span className="tagbtn__count">{count}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {filtering && (
          <button
            type="button"
            className="toolbar__button"
            onClick={() => {
              clearQuery();
              setTags([]);
            }}
          >
            Clear
          </button>
        )}
        <button type="button" className="toolbar__button" onClick={() => setFitRequest((n) => n + 1)}>
          Fit
        </button>
        <span className="toolbar__stats" aria-live="polite">
          {shown === graph.nodes.length ? `${shown} nodes` : `${shown} of ${graph.nodes.length} nodes`}
          {laying ? " · laying out…" : timing !== null && ` · ${Math.round(timing.paint || timing.layout)} ms`}
        </span>
      </div>
      <div className="graph__body">
        {shown === 0 ? (
          <p className="graph__empty">No nodes match the filter.</p>
        ) : (
          <GraphCanvas
            layout={layout}
            selected={selected}
            lineage={lin}
            onSelect={setSelected}
            onOpen={open}
            fitRequest={fitRequest}
          />
        )}
        {layoutError !== null && graphError(layoutError)}
        {selected !== null && (
          <NodePanel
            id={selected}
            nodes={graph.nodes}
            revision={graph}
            width={panelWidth}
            onResize={setPanelWidth}
            onSelect={setSelected}
            onClose={() => setSelected(null)}
          />
        )}
      </div>
    </section>
  );
}

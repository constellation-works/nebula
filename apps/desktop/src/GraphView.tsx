import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import { GraphCanvas } from "./GraphCanvas";
import { fromElkLayout, lineage, matchingIds, tagCounts, toElkGraph, type Layout } from "./layout";
import { layoutGraph } from "./layoutClient";
import { NodePanel } from "./NodePanel";
import { useGraph } from "./useGraph";

const EMPTY: Layout = { width: 0, height: 0, nodes: [], edges: [] };
const PANEL_DEFAULT = 380;
const FILTER_DEBOUNCE_MS = 250;

/**
 * The whole corpus as a layered DAG, with filter highlighting and a panel
 * for one node. Layout stays fixed across search and tag changes.
 */
export function GraphView({ active = true }: { active?: boolean }) {
  const { graph, error, loaded, refresh } = useGraph();
  const [query, setQuery] = useState("");
  const [appliedQuery, setAppliedQuery] = useState("");
  const [searchResult, setSearchResult] = useState<{ graph: typeof graph; query: string; ids: Set<string> } | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [shortcut, setShortcut] = useState<string | null>(null);
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
  const [focus, setFocus] = useState<{ id: string; serial: number } | null>(null);
  const [cursor, setCursor] = useState(-1);
  const [isolated, setIsolated] = useState(false);
  const started = useRef(0);

  useEffect(() => {
    if (query === appliedQuery) return;
    const timer = setTimeout(() => setAppliedQuery(query), FILTER_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query, appliedQuery]);

  useEffect(() => {
    if (!active) return;
    let live = true;
    void api.captureShortcut().then((value) => { if (live) setShortcut(value); }).catch(() => {});
    return () => { live = false; };
  }, [active]);

  useEffect(() => {
    if (graph === null || appliedQuery.trim() === "") return;
    let live = true;
    setSearchError(null);
    void api.graphSearch(appliedQuery).then(
      (ids) => {
        if (!live) return;
        setSearchResult({ graph, query: appliedQuery, ids: new Set(ids) });
        setSearchError(null);
      },
      (e: unknown) => { if (live) setSearchError(`Search failed: ${String(e)}`); },
    );
    return () => { live = false; };
  }, [graph, appliedQuery]);

  const counts = useMemo(() => (graph === null ? [] : tagCounts(graph.nodes)), [graph]);
  const searchIds = appliedQuery.trim() === "" ? null : searchResult?.graph === graph && searchResult.query === appliedQuery ? searchResult.ids : null;
  const searchPending = query !== appliedQuery || (appliedQuery.trim() !== "" && searchIds === null);
  const matches = useMemo(() => graph === null || searchPending ? null : matchingIds(graph, { searchIds, tags }), [graph, searchIds, searchPending, tags]);
  const matchList = useMemo(() => graph?.nodes.filter((n) => matches?.has(n.id)).map((n) => n.id) ?? [], [graph, matches]);
  useEffect(() => { setCursor(-1); }, [matchList]);

  // Filtering changes only opacity. The full graph is laid out on refetch.
  useEffect(() => {
    if (graph === null) return;
    if (graph.nodes.length === 0) {
      setLayout(EMPTY);
      setLaying(false);
      return;
    }
    let live = true;
    started.current = performance.now();
    setLaying(true);
    layoutGraph(toElkGraph(graph)).then(
      (laid) => {
        if (!live) return;
        const ms = performance.now() - started.current;
        setLayout(fromElkLayout(laid, graph));
        setTiming({ layout: ms, paint: 0 });
        setLayoutError(null);
        setLaying(false);
      },
      (e: unknown) => {
        if (!live) return;
        setLayoutError(String(e));
        setLaying(false);
      },
    );
    return () => {
      live = false;
    };
  }, [graph]);

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

  // A node that left the corpus cannot stay selected.
  useEffect(() => {
    if (selected !== null && graph !== null && !graph.nodes.some((n) => n.id === selected)) {
      setSelected(null);
    }
  }, [graph, selected]);

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSelected(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

  const lin = useMemo(
    () => (selected === null || graph === null ? null : lineage(graph.edges, selected)),
    [graph, selected],
  );
  const isolatedIds = useMemo(() => isolated && selected !== null && lin !== null ? new Set([selected, ...lin.ancestors, ...lin.descendants]) : null, [isolated, selected, lin]);
  useEffect(() => { if (selected === null) setIsolated(false); }, [selected]);

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
    setTags((cur) => (cur.includes(t) ? cur.filter((x) => x !== t) : [...cur, t]));
  };
  const clearQuery = () => {
    setQuery("");
    setAppliedQuery("");
    setSearchError(null);
  };
  const step = (direction: number) => {
    if (matchList.length === 0) return;
    const next = cursor < 0
      ? direction > 0 ? 0 : matchList.length - 1
      : (cursor + direction + matchList.length) % matchList.length;
    const id = matchList[next]!;
    setCursor(next);
    setIsolated(false);
    setSelected(id);
    setFocus((previous) => ({ id, serial: (previous?.serial ?? 0) + 1 }));
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
  const shown = matches?.size ?? graph.nodes.length;

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
          placeholder="Search id, title, body, status"
          aria-label="Search graph"
          value={query}
          onChange={(e) => {
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
        {filtering && !searchPending && (
          <span className="toolbar__matches">
            <button type="button" className="toolbar__button" onClick={() => step(-1)} disabled={matchList.length === 0} aria-label="Previous match">←</button>
            <span aria-live="polite">{cursor < 0 ? 0 : cursor + 1} / {matchList.length}</span>
            <button type="button" className="toolbar__button" onClick={() => step(1)} disabled={matchList.length === 0} aria-label="Next match">→</button>
          </span>
        )}
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
        <button type="button" className="toolbar__button" aria-pressed={isolated} disabled={selected === null} onClick={() => setIsolated((value) => !value)}>
          Lineage only
        </button>
        <span className="toolbar__stats" aria-live="polite">
          {searchPending ? searchError === null ? "searching…" : "search unavailable" : shown === graph.nodes.length ? `${shown} nodes` : `${shown} of ${graph.nodes.length} nodes`}
          {laying ? " · laying out…" : timing !== null && ` · ${Math.round(timing.paint || timing.layout)} ms`}
        </span>
      </div>
      <div className="graph__hint">Drag or scroll to pan · Ctrl+scroll to zoom · Double-click a node to open · Tab to cards, arrow keys follow edges, Enter selects · Orange ancestors · Green descendants · Capture: {shortcut ?? "…"}</div>
      <div className="graph__body">
        {searchError !== null && <div className="graph__error" role="alert">{searchError}</div>}
        <GraphCanvas
            layout={layout}
            selected={selected}
            lineage={lin}
            matches={filtering ? matches : null}
            isolatedIds={isolatedIds}
            focus={focus}
            onSelect={setSelected}
            onOpen={open}
            fitRequest={fitRequest}
        />
        {filtering && !searchPending && shown === 0 && <p className="graph__no-matches">No nodes match the filter.</p>}
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

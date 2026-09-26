import { memo, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";
import { MAX_CHIPS, truncate, type DrawnEdge, type Layout, type Lineage, type PlacedNode, type Point } from "./layout";

/** Where the drawing sits in the canvas: a translate then a scale. */
export interface Viewport {
  x: number;
  y: number;
  k: number;
}

interface Props {
  layout: Layout;
  selected: string | null;
  /** The selected node's lineage, so the rest can be dimmed. */
  lineage: Lineage | null;
  /** Active filter matches; null means no filter. */
  matches: ReadonlySet<string> | null;
  /** IDs to show when lineage isolation is active. */
  isolatedIds: ReadonlySet<string> | null;
  /** A toolbar navigation request, including repeated visits to one node. */
  focus: { id: string; serial: number } | null;
  onSelect: (id: string | null) => void;
  onOpen: (id: string) => void;
  /** Bumped by the toolbar's Fit button; also fits once on the first layout. */
  fitRequest: number;
}

const MIN_ZOOM = 0.15;
const MAX_ZOOM = 3;
const FIT_PADDING = 40;
/** Below this many pixels a mouse-down/up is a click, not a pan. */
const DRAG_SLOP = 4;

/** The transform that shows the whole layout, never larger than life. */
function fit(layout: Layout, width: number, height: number): Viewport {
  if (width <= 0 || height <= 0 || layout.width <= 0 || layout.height <= 0) {
    return { x: FIT_PADDING, y: FIT_PADDING, k: 1 };
  }
  const k = Math.min(
    1,
    (width - 2 * FIT_PADDING) / layout.width,
    (height - 2 * FIT_PADDING) / layout.height,
  );
  return {
    x: (width - layout.width * k) / 2,
    y: (height - layout.height * k) / 2,
    k,
  };
}

const pathOf = (points: Point[]): string =>
  points.map((p, i) => `${i === 0 ? "M" : "L"}${p.x.toFixed(1)} ${p.y.toFixed(1)}`).join(" ");

/** Which tint an edge takes once a node is selected, or none to dim it. */
function edgeTint(e: DrawnEdge, selected: string, lin: Lineage): "ancestor" | "descendant" | "near" | null {
  if (e.type === "contradicts") {
    return e.from === selected || e.to === selected ? "near" : null;
  }
  // Genealogy runs child (`from`) → parent (`to`).
  const childUp = e.from === selected || lin.ancestors.has(e.from);
  if (childUp && lin.ancestors.has(e.to)) return "ancestor";
  const parentDown = e.to === selected || lin.descendants.has(e.to);
  if (parentDown && lin.descendants.has(e.from)) return "descendant";
  return null;
}

function nodeTint(id: string, selected: string, lin: Lineage): "selected" | "ancestor" | "descendant" | null {
  if (id === selected) return "selected";
  if (lin.ancestors.has(id)) return "ancestor";
  if (lin.descendants.has(id)) return "descendant";
  return null;
}

/** Roughly how wide a chip's text runs at the chip font size. */
const chipWidth = (text: string): number => Math.round(text.length * 5.4 + 12);
const CHIP_CHARS = 14;

const Edges = memo(function Edges({
  edges,
  selected,
  lineage,
}: {
  edges: DrawnEdge[];
  selected: string | null;
  lineage: Lineage | null;
}) {
  return (
    <g className="edges">
      {edges.map((e) => {
        const tint = selected !== null && lineage !== null ? edgeTint(e, selected, lineage) : undefined;
        const kind = e.type === "contradicts" ? "contradicts" : "genealogy";
        const cls = [
          "edge",
          `edge--${kind}`,
          tint === undefined ? "" : tint === null ? "edge--dim" : `edge--${tint}`,
        ]
          .filter(Boolean)
          .join(" ");
        const marker =
          kind === "genealogy" ? `url(#arrow${tint === undefined || tint === null ? "" : `-${tint}`})` : undefined;
        return (
          <path
            key={`${e.from}>${e.to}:${e.type}`}
            className={cls}
            d={pathOf(e.points)}
            markerEnd={marker}
            data-type={e.type}
          />
        );
      })}
    </g>
  );
});

const Nodes = memo(function Nodes({
  nodes,
  selected,
  lineage,
  matches,
  onSelect,
  onOpen,
}: {
  nodes: PlacedNode[];
  selected: string | null;
  lineage: Lineage | null;
  matches: ReadonlySet<string> | null;
  onSelect: (id: string) => void;
  onOpen: (id: string) => void;
}) {
  return (
    <g className="nodes">
      {nodes.map((n) => {
        const tint = selected !== null && lineage !== null ? nodeTint(n.id, selected, lineage) : undefined;
        const cls = [
          "node",
          `node--${n.status}`,
          tint === undefined ? "" : tint === null ? "node--dim" : `node--${tint}`,
          matches !== null && !matches.has(n.id) ? "node--filter-dim" : "",
        ]
          .filter(Boolean)
          .join(" ");
        const shown = n.tags.slice(0, MAX_CHIPS);
        const more = n.tags.length - shown.length;
        let cx = 12;
        return (
          <g
            key={n.id}
            className={cls}
            transform={`translate(${n.x} ${n.y})`}
            role="button"
            tabIndex={0}
            aria-label={`${n.title}, ${n.status}`}
            aria-pressed={n.id === selected}
            data-id={n.id}
            data-status={n.status}
            onClick={(e) => {
              e.stopPropagation();
              onSelect(n.id);
            }}
            onDoubleClick={(e) => {
              e.stopPropagation();
              onOpen(n.id);
            }}
          >
            <title>{n.title}</title>
            <rect className="node__card" width={n.width} height={n.height} rx={8} />
            <rect className="node__stripe" width={4} height={n.height} rx={2} />
            <text className="node__title" x={14} y={22}>
              {truncate(n.title)}
            </text>
            <g className="node__tags" transform={`translate(0 ${n.height - 22})`}>
              {shown.map((t) => {
                const label = t.length > CHIP_CHARS ? `${t.slice(0, CHIP_CHARS - 1)}…` : t;
                const w = chipWidth(label);
                const x = cx;
                cx += w + 6;
                return (
                  <g key={t} className="chip" transform={`translate(${x} 0)`}>
                    <rect width={w} height={16} rx={8} />
                    <text x={w / 2} y={11.5} textAnchor="middle">
                      {label}
                    </text>
                  </g>
                );
              })}
              {more > 0 && (
                <text className="node__more" x={cx + 2} y={11.5}>
                  +{more}
                </text>
              )}
            </g>
            {n.reopens && (
              <g className="node__reopens" transform={`translate(${n.width - 14} 12)`}>
                <title>reopens a refuted node</title>
                <circle r={5} />
                <text y={3} textAnchor="middle">
                  ↺
                </text>
              </g>
            )}
          </g>
        );
      })}
    </g>
  );
});

/**
 * The drawing: SVG, one `<g>` per node and one `<path>` per edge, under a
 * transform that the mouse moves. Drag and plain wheel pan; ctrl-wheel zooms about the cursor,
 * a click on empty canvas clears the selection.
 */
export function GraphCanvas({ layout, selected, lineage, matches, isolatedIds, focus, onSelect, onOpen, fitRequest }: Props) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [view, setView] = useState<Viewport>({ x: FIT_PADDING, y: FIT_PADDING, k: 1 });
  const [panning, setPanning] = useState(false);
  const visibleNodes = isolatedIds === null ? layout.nodes : layout.nodes.filter((n) => isolatedIds.has(n.id));
  const visibleEdges = isolatedIds === null ? layout.edges : layout.edges.filter((e) => isolatedIds.has(e.from) && isolatedIds.has(e.to));
  const viewRef = useRef(view);
  viewRef.current = view;
  const fitted = useRef(-1);
  const drag = useRef<{ x: number; y: number; ox: number; oy: number; moved: boolean } | null>(null);

  // Fit on the first layout that has size, then only when asked, so a refetch
  // or a filter change leaves the viewport where the user put it.
  useEffect(() => {
    if (fitted.current === fitRequest) return;
    if (layout.nodes.length === 0) return;
    fitted.current = fitRequest;
    const svg = svgRef.current;
    setView(fit(layout, svg?.clientWidth ?? 0, svg?.clientHeight ?? 0));
  }, [layout, fitRequest]);

  useEffect(() => {
    if (focus === null) return;
    const node = layout.nodes.find((n) => n.id === focus.id);
    if (node === undefined) return;
    const svg = svgRef.current;
    const k = Math.max(1, viewRef.current.k);
    setView({
      x: (svg?.clientWidth ?? 0) / 2 - (node.x + node.width / 2) * k,
      y: (svg?.clientHeight ?? 0) / 2 - (node.y + node.height / 2) * k,
      k,
    });
  }, [focus, layout]);

  // React registers wheel listeners passively, and a passive listener cannot
  // stop the page from scrolling; attach the real one.
  useEffect(() => {
    const svg = svgRef.current;
    if (svg === null) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? svg.clientHeight : 1;
      const v = viewRef.current;
      if (!e.ctrlKey) {
        setView({ ...v, x: v.x - e.deltaX * unit, y: v.y - e.deltaY * unit });
        return;
      }
      const rect = svg.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;
      const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.k * Math.exp(-e.deltaY * unit * 0.0025)));
      const r = k / v.k;
      setView({ x: mx - (mx - v.x) * r, y: my - (my - v.y) * r, k });
    };
    svg.addEventListener("wheel", onWheel, { passive: false });
    return () => svg.removeEventListener("wheel", onWheel);
  }, []);

  function onMouseDown(e: ReactMouseEvent<SVGSVGElement>) {
    if (e.button !== 0) return;
    const v = viewRef.current;
    drag.current = { x: e.clientX, y: e.clientY, ox: v.x, oy: v.y, moved: false };
  }

  function onMouseMove(e: ReactMouseEvent<SVGSVGElement>) {
    const d = drag.current;
    if (d === null) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (!d.moved && Math.hypot(dx, dy) < DRAG_SLOP) return;
    if (!d.moved) {
      d.moved = true;
      setPanning(true);
    }
    setView({ x: d.ox + dx, y: d.oy + dy, k: viewRef.current.k });
  }

  function onMouseUp(e: ReactMouseEvent<SVGSVGElement>) {
    const moved = drag.current?.moved ?? false;
    endDrag();
    // A pan that ends on empty canvas is not a click; node clicks are the
    // node's own business.
    if (!moved && !(e.target as Element).closest(".node")) onSelect(null);
  }

  function endDrag() {
    drag.current = null;
    setPanning(false);
  }

  function onCardKeyDown(e: ReactKeyboardEvent<SVGSVGElement>) {
    const card = (e.target as Element).closest<SVGGElement>(".node");
    const id = card?.dataset.id;
    if (id === undefined) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onSelect(id);
      return;
    }
    const direction = { ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right" }[e.key];
    if (direction === undefined) return;
    e.preventDefault();
    const current = visibleNodes.find((n) => n.id === id);
    if (current === undefined) return;
    const cx = current.x + current.width / 2;
    const cy = current.y + current.height / 2;
    const neighbours = new Set(visibleEdges.flatMap((edge) => edge.from === id ? [edge.to] : edge.to === id ? [edge.from] : []));
    const next = visibleNodes
      .filter((n) => neighbours.has(n.id))
      .map((n) => ({ node: n, dx: n.x + n.width / 2 - cx, dy: n.y + n.height / 2 - cy }))
      .filter(({ dx, dy }) => direction === "up" ? dy < 0 : direction === "down" ? dy > 0 : direction === "left" ? dx < 0 : dx > 0)
      .sort((a, b) => a.dx * a.dx + a.dy * a.dy - b.dx * b.dx - b.dy * b.dy)[0]?.node;
    if (next === undefined) return;
    const target = [...(svgRef.current?.querySelectorAll<SVGGElement>(".node") ?? [])]
      .find((element) => element.dataset.id === next.id);
    target?.focus();
    const k = viewRef.current.k;
    setView({
      x: (svgRef.current?.clientWidth ?? 0) / 2 - (next.x + next.width / 2) * k,
      y: (svgRef.current?.clientHeight ?? 0) / 2 - (next.y + next.height / 2) * k,
      k,
    });
  }

  return (
    <svg
      ref={svgRef}
      className={`canvas${panning ? " canvas--panning" : ""}`}
      role="group"
      aria-label="Graph"
      onKeyDown={onCardKeyDown}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
      onMouseLeave={endDrag}
    >
      <defs>
        {["", "-ancestor", "-descendant"].map((suffix) => (
          <marker
            key={suffix}
            id={`arrow${suffix}`}
            className={`arrow${suffix ? ` arrow-${suffix.slice(1)}` : ""}`}
            viewBox="0 0 10 10"
            refX={9}
            refY={5}
            markerWidth={7}
            markerHeight={7}
            orient="auto-start-reverse"
          >
            <path d="M0 0 L10 5 L0 10 z" />
          </marker>
        ))}
      </defs>
      <g className="scene" transform={`translate(${view.x} ${view.y}) scale(${view.k})`}>
        <Edges edges={visibleEdges} selected={selected} lineage={lineage} />
        <Nodes nodes={visibleNodes} selected={selected} lineage={lineage} matches={matches} onSelect={onSelect} onOpen={onOpen} />
      </g>
    </svg>
  );
}

// „Mapa": projects as islands on a pannable canvas, their topics as rows
// along a spine, open tasks on expanding, and the connections between topics
// and tasks written as references in summaries and notes (lib/refs.ts).
// Rows are real buttons in document order over an SVG layer (spines and the
// focused node's curves). Layout: lib/mapLayout.ts (pure; zoom only scales).

import { Hourglass, Maximize, Minus, Plus } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import type { Project, Task, Topic, TopicStatus } from "../api";
import { TopicStatusPill, useCopyContext } from "../components/TopicItem";
import { Button, Segmented } from "../components/ui";
import { formatAge } from "../lib/dates";
import { describeError } from "../lib/errors";
import { journalOf, partitionProjects } from "../lib/group";
import { TASK_STATUS_LABEL, TOPIC_STATUS_LABEL } from "../lib/labels";
import { fitView, layoutMap, routeCurve, type LayoutRow, type MapLayout } from "../lib/mapLayout";
import { appendRefLine, buildGraph, counts, neighbours, removeRefLine, taskKey, topicKey, waitChain, type EdgeKind, type Graph, type NodeKey } from "../lib/refs";
import { plural } from "../lib/text";
import { buildToday, firstSentence } from "../lib/today";
import type { ViewProps } from "../router/registry";
import { links, navigate } from "../router/router";
import { driftStore, useDrift, useWaiting, waitingStore } from "../store/signals";
import { readPref, writePref } from "../store/ui";
import { updateStore, useUpdates } from "../store/updates";
import { useVault, useView } from "../store/vault";
import "./map.css";

const VIEW_PREF = "montybrain.map.view";
const FILTER_PREF = "montybrain.map.filters";
const MAX_CURVES = 8;

interface Filters {
  project: string | null;
  attention: boolean;
  done: boolean;
}

function readJson<T>(key: string, fallback: T): T {
  try {
    const v = readPref(key);
    return v ? { ...fallback, ...(JSON.parse(v) as T) } : fallback;
  } catch {
    return fallback;
  }
}

/** `projekt/slug` → topic key, `^id` → task key (the `focus` query). */
export function focusKey(q: string | null): NodeKey | null {
  if (!q) return null;
  if (q.startsWith("^")) return `k:${q.slice(1)}`;
  return q.includes("/") ? `t:${q}` : null;
}

/** The query form of a node key. */
export function focusParam(key: NodeKey): string {
  return key.startsWith("t:") ? key.slice(2) : key.startsWith("k:") ? `^${key.slice(2)}` : key;
}

const statusWord = (s: TopicStatus) => TOPIC_STATUS_LABEL[s].toLowerCase();

export function MapView({ route }: ViewProps) {
  const view = useView();
  const snap = view.snapshot;
  const { notify } = useVault();
  const waiting = useWaiting();
  const drift = useDrift();
  const updates = useUpdates();
  useEffect(() => {
    void waitingStore.start().catch(() => {});
    void driftStore.start().catch(() => {});
    void updateStore.start().catch(() => {});
  }, []);

  const [filters, setFiltersState] = useState<Filters>(() => {
    const f = readJson<Filters>(FILTER_PREF, { project: null, attention: false, done: false });
    return route.query.get("attention") === "1" ? { ...f, attention: true } : f;
  });
  const setFilters = (f: Filters) => {
    setFiltersState(f);
    writePref(FILTER_PREF, JSON.stringify(f));
  };
  const [focus, setFocus] = useState<NodeKey | null>(() => focusKey(route.query.get("focus")));
  const [expanded, setExpanded] = useState<Record<string, string | undefined>>(() => {
    const e = route.query.get("expand");
    if (!e || !e.includes("/")) return {};
    const [p, s] = e.split("/") as [string, string];
    return { [p]: s };
  });
  useEffect(() => {
    const k = focusKey(route.query.get("focus"));
    if (k) setFocus(k);
  }, [route.query.get("focus")]);

  // What is shown: active projects (all with "Pokaż zakończone"), in sidebar order.
  const { visible, archived } = partitionProjects(snap.projects);
  const projects = [...visible, ...archived].filter((p) => (filters.done || p.state === "active") && (!filters.project || p.slug === filters.project));
  const topicsOf = (p: string) => snap.topics.filter((t) => t.project === p && (filters.done || t.status !== "done"));
  const openTasksOf = (p: string, t: string) =>
    snap.tasks.filter((x) => x.status !== "done" && x.location.kind === "topic" && x.location.project === p && x.location.topic === t);

  // The layout depends only on what it reads: projects, topic slugs and dates, the expanded topics' open tasks.
  const layoutKey = JSON.stringify([
    projects.map((p) => [p.slug, topicsOf(p.slug).map((t) => [t.slug, t.created])]),
    Object.entries(expanded).map(([p, t]) => [p, t, t ? openTasksOf(p, t).map(taskKey) : []]),
  ]);
  const layout = useMemo<MapLayout>(
    () =>
      layoutMap({
        projects: projects.map((p) => ({ slug: p.slug, topics: topicsOf(p.slug).map((t) => ({ slug: t.slug, created: t.created })) })),
        expanded,
        openTasks: (p, t) => openTasksOf(p, t).map(taskKey),
      }),
    [layoutKey],
  );
  const graph = useMemo<Graph>(() => buildGraph(snap), [snap]);
  const linkCounts = useMemo(() => counts(graph), [graph]);
  const today = buildToday({ snapshot: snap, today: view.today, waiting: waiting.list, drift: drift.list, updates: Object.values(updates.byProject) });
  const attention = new Set(today.attention.map((r) => (r.kind === "project" ? `p:${r.project}` : r.kind === "topic" ? `t:${r.project}/${r.topic.slug}` : r.task.id ? `k:${r.task.id}` : r.key)));

  const topicByKey = useMemo(() => new Map(snap.topics.map((t) => [topicKey(t.project, t.slug), t])), [snap]);
  const taskByKey = useMemo(() => new Map(snap.tasks.map((t) => [taskKey(t), t])), [snap]);
  const rowByKey = new Map(layout.rows.map((r) => [r.key, r]));
  const islandOf = new Map(layout.islands.map((i) => [i.project, i]));

  // --- pan & zoom ---------------------------------------------------------------------
  const canvas = useRef<HTMLDivElement>(null);
  const [vp, setVp] = useState<{ x: number; y: number; z: number } | null>(() => {
    const v = readJson<{ x: number; y: number; z: number } | null>(VIEW_PREF, null);
    return v && Number.isFinite(v.z) ? v : null;
  });
  // The latest view, for effects that run before a re-render (fit, then reveal).
  const vRef = useRef(vp);
  vRef.current = vp ?? vRef.current;
  const setView = (v: { x: number; y: number; z: number }) => {
    vRef.current = v;
    setVp(v);
    writePref(VIEW_PREF, JSON.stringify(v));
  };
  const fit = () => {
    const el = canvas.current;
    if (el) setView(fitView(layout, { w: el.clientWidth, h: el.clientHeight }));
  };
  useLayoutEffect(() => {
    if (!vp) fit();
  }, []);
  const v = vp ?? { x: 0, y: 0, z: 1 };
  const zoomAt = (factor: number, cx: number, cy: number) => {
    const z = Math.min(2, Math.max(0.2, v.z * factor));
    const k = z / v.z;
    setView({ z, x: cx - (cx - v.x) * k, y: cy - (cy - v.y) * k });
  };
  // Wheel: pan; with Ctrl (pinch on a trackpad): zoom around the pointer.
  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) zoomAt(Math.exp(-e.deltaY / 300), e.clientX - r.left, e.clientY - r.top);
      else setView({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });
  const drag = useRef<{ x: number; y: number; vx: number; vy: number; moved: boolean } | null>(null);
  const onPointerDown = (e: ReactPointerEvent) => {
    if ((e.target as HTMLElement).closest("button, a, input, select, .map-panel")) return;
    drag.current = { x: e.clientX, y: e.clientY, vx: v.x, vy: v.y, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: ReactPointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
    setVp({ z: v.z, x: d.vx + dx, y: d.vy + dy });
  };
  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.moved && vp) writePref(VIEW_PREF, JSON.stringify(vp));
    else if (d && !d.moved) setFocus(null);
  };

  /** Brings a row into view (pan only) when it is outside. */
  const reveal = (key: NodeKey, closer = false) => {
    const r = rowByKey.get(key) ?? (taskByKey.get(key) ? rowByKey.get(parentTopic(taskByKey.get(key)!) ?? "") : undefined);
    const el = canvas.current;
    if (!r || !el) return;
    const cur = vRef.current ?? v;
    const w = el.clientWidth - (focus || key ? panelWidth(el.clientWidth) : 0);
    // Too small to read: come closer (zoom only scales; positions stay).
    let z = closer && cur.z < 0.6 ? 0.8 : cur.z;
    // In a narrow window the row should still fit beside the panel.
    if (closer && r.rect.w * z > w - 16) z = Math.max(0.5, (w - 16) / r.rect.w);
    const sx = r.rect.x * z + cur.x;
    const sy = r.rect.y * z + cur.y;
    if (z !== cur.z || sx < 0 || sy < 0 || sx + r.rect.w * z > w || sy + r.rect.h * z > el.clientHeight) {
      const x = r.rect.w * z > w - 16 ? 8 - r.rect.x * z : w / 2 - (r.rect.x + r.rect.w / 2) * z;
      setView({ z, x, y: el.clientHeight / 2 - (r.rect.y + r.rect.h / 2) * z });
    }
  };
  useEffect(() => {
    if (focus) reveal(focus);
  }, [focus]);
  // Arriving with `?focus=` (⌘K, Dziś, the project view): close enough to read.
  const arrived = route.query.get("focus");
  useEffect(() => {
    const k = focusKey(arrived);
    if (k) requestAnimationFrame(() => reveal(k, true));
  }, [arrived]);

  // --- focus, neighbours, curves ------------------------------------------------------
  const focusTopic = focus ? topicByKey.get(focus) : undefined;
  const focusTask = focus ? taskByKey.get(focus) : undefined;
  const near = focus ? neighbours(graph, focus) : [];
  const nearKeys = new Set(near.map((n) => n.other));
  const anchor = (key: NodeKey): LayoutRow | undefined => {
    const r = rowByKey.get(key);
    if (r) return r;
    const t = taskByKey.get(key);
    return t ? rowByKey.get(parentTopic(t) ?? "") : undefined;
  };
  const drawn = focus
    ? near
        .map((n) => ({ n, a: anchor(focus), b: anchor(n.other) }))
        .filter((x) => x.a && x.b && x.a.key !== x.b.key)
        .slice(0, MAX_CURVES)
    : [];
  const lit = (r: LayoutRow) => {
    if (!focus) return true;
    if (r.key === focus || nearKeys.has(r.key)) return true;
    if (r.kind === "project") return r.project === focusTopic?.project || [...nearKeys].some((k) => k.startsWith(`t:${r.project}/`));
    // The focused topic's own tasks stay clear.
    if ((r.kind === "task" || r.kind === "more") && focus === `t:${r.project}/${r.topic}`) return true;
    return false;
  };

  // --- keyboard -----------------------------------------------------------------------
  const order = layout.rows.filter((r) => r.kind !== "more").map((r) => r.key);
  const focusRow = (key: NodeKey) => {
    setFocus(key);
    requestAnimationFrame(() => canvas.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`)?.focus({ preventScroll: true }));
  };
  const toggleExpand = (r: LayoutRow, open?: boolean) => {
    if (r.kind !== "topic" || !r.topic) return;
    const isOpen = expanded[r.project] === r.topic;
    const next = open ?? !isOpen;
    setExpanded({ ...expanded, [r.project]: next ? r.topic : undefined });
  };
  const openInProject = (key: NodeKey) => {
    const t = topicByKey.get(key);
    if (t) return navigate(links.project(t.project, t.slug));
    const k = taskByKey.get(key);
    if (k && k.location.kind !== "inbox") navigate(links.project(k.location.project, k.location.kind === "topic" ? k.location.topic : undefined));
  };
  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).closest("input, textarea, select, .map-panel, .menu")) return;
    const i = focus ? order.indexOf(focus) : -1;
    const r = focus ? rowByKey.get(focus) : undefined;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = order.length;
      if (!n) return;
      const next = i < 0 ? 0 : (i + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
      focusRow(order[next]!);
    } else if (e.key === "ArrowRight" && r) {
      e.preventDefault();
      toggleExpand(r, true);
    } else if (e.key === "ArrowLeft" && r) {
      e.preventDefault();
      if (r.kind === "task" && r.topic) focusRow(`t:${r.project}/${r.topic}`);
      else toggleExpand(r, false);
    } else if (e.key === "Enter" && focus) {
      e.preventDefault();
      if (r?.kind === "project") navigate(links.project(r.project));
      else openInProject(focus);
    } else if (e.key === "Escape") {
      setFocus(null);
    } else if (e.key === "0") {
      e.preventDefault();
      fit();
    } else if (e.key === "+" || e.key === "=") {
      zoomAt(1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2);
    } else if (e.key === "-") {
      zoomAt(1 / 1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2);
    }
  };

  // A click focuses the button before its click event: that focus must not count as the first click.
  const pressed = useRef<string | null>(null);
  const onRowClick = (r: LayoutRow) => {
    pressed.current = null;
    if (r.kind === "more" && r.topic) return navigate(links.project(r.project, r.topic));
    if (r.kind === "project") return setFocus(focus === r.key ? null : r.key);
    // A second click on a focused topic expands (or collapses) it.
    if (focus === r.key) toggleExpand(r);
    else setFocus(r.key);
  };

  const projectOf = (slug: string) => snap.projects.find((p) => p.slug === slug);
  const empty = layout.rows.length === 0;

  return (
    <div className="view map-view">
      <div className="map-toolbar" role="toolbar" aria-label="Mapa: widok i filtry">
        <h1 className="map-title">Mapa</h1>
        <select className="field select map-select" aria-label="Projekt" value={filters.project ?? ""} onChange={(e) => setFilters({ ...filters, project: e.target.value || null })}>
          <option value="">Wszystkie projekty</option>
          {[...visible, ...archived]
            .filter((p) => filters.done || p.state === "active")
            .map((p) => (
              <option key={p.slug} value={p.slug}>
                {p.name}
              </option>
            ))}
        </select>
        <label className="map-check">
          <input type="checkbox" checked={filters.attention} onChange={(e) => setFilters({ ...filters, attention: e.target.checked })} /> Wymaga uwagi
        </label>
        <label className="map-check">
          <input type="checkbox" checked={filters.done} onChange={(e) => setFilters({ ...filters, done: e.target.checked })} /> Pokaż zakończone
        </label>
        <span className="map-zoom">
          <button type="button" className="icon-btn" aria-label="Pomniejsz" title="Pomniejsz (−)" onClick={() => zoomAt(1 / 1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2)}>
            <Minus size={14} />
          </button>
          <button type="button" className="icon-btn" aria-label="Powiększ" title="Powiększ (+)" onClick={() => zoomAt(1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2)}>
            <Plus size={14} />
          </button>
          <button type="button" className="btn btn-quiet map-fit" title="Dopasuj widok (0)" onClick={fit}>
            <Maximize size={13} /> Dopasuj
          </button>
        </span>
      </div>

      <div
        ref={canvas}
        className={`map-canvas${focus ? " focused" : ""}${filters.attention ? " attention" : ""}`}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        aria-label="Mapa projektów i tematów. Strzałki: wiersze; Enter: otwórz; strzałka w prawo: zadania; Esc: koniec wyboru; 0: dopasuj widok."
        role="application"
      >
        {empty ? (
          <p className="empty-text map-empty">Brak aktywnych projektów z tematami. Włącz „Pokaż zakończone” albo dodaj temat w widoku projektu.</p>
        ) : (
          <div className="map-world" style={{ transform: `translate(${v.x}px, ${v.y}px) scale(${v.z})`, width: layout.width, height: layout.height }}>
            {layout.islands.map((i) => (
              <div key={i.project} className="map-island" style={{ left: i.rect.x, top: i.rect.y, width: i.rect.w, height: i.rect.h, ...(projectOf(i.project)?.color ? { "--accent": projectOf(i.project)!.color } : {}) } as React.CSSProperties} />
            ))}
            <svg className="map-lines" width={layout.width} height={layout.height} aria-hidden>
              <defs>
                <marker id="map-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                  <path d="M0,0 L8,4 L0,8 z" className="map-arrow" />
                </marker>
              </defs>
              {layout.islands.map((i) => (
                <line key={i.project} className="map-spine" x1={i.spine.x} x2={i.spine.x} y1={i.spine.y1} y2={i.spine.y2} />
              ))}
              {drawn.map(({ n, a, b }, lane) => {
                const ia = islandOf.get(a!.project)!;
                const ib = islandOf.get(b!.project)!;
                const from = n.outgoing ? a! : b!;
                const to = n.outgoing ? b! : a!;
                const fi = n.outgoing ? ia : ib;
                const ti = n.outgoing ? ib : ia;
                return (
                  <path
                    key={`${n.other}-${n.edge.kind}-${lane}`}
                    className={`map-curve k-${n.edge.kind}`}
                    d={routeCurve(from.rect, to.rect, fi.rect, ti.rect, lane)}
                    markerEnd={n.edge.kind === "waits" ? "url(#map-arrow)" : undefined}
                  />
                );
              })}
            </svg>
            {layout.rows.map((r) => (
              <MapRow
                key={r.key}
                row={r}
                snap={snap}
                topic={r.kind === "topic" ? topicByKey.get(r.key) : undefined}
                task={r.kind === "task" ? taskByKey.get(r.key) : undefined}
                project={r.kind === "project" ? projectOf(r.project) : undefined}
                openCount={r.kind === "topic" && r.topic ? openTasksOf(r.project, r.topic).length : 0}
                links={linkCounts.get(r.key)}
                attention={attention.has(r.key)}
                focused={focus === r.key}
                lit={lit(r) && (!filters.attention || attention.has(r.key) || r.kind === "task")}
                expanded={r.kind === "topic" && expanded[r.project] === r.topic}
                onClick={() => onRowClick(r)}
                onPress={() => (pressed.current = r.key)}
                onFocus={() => pressed.current !== r.key && focus !== r.key && r.kind !== "more" && setFocus(r.key)}
              />
            ))}
          </div>
        )}
        {(focusTopic || focusTask) && (
          <MapPanel
            key={focus!}
            nodeKey={focus!}
            topic={focusTopic}
            task={focusTask}
            graph={graph}
            topicByKey={topicByKey}
            taskByKey={taskByKey}
            drawn={drawn.length}
            onClose={() => setFocus(null)}
            onFocus={(k) => focusRow(k)}
            onOpen={() => openInProject(focus!)}
            notify={notify}
          />
        )}
      </div>
    </div>
  );
}

function parentTopic(t: Task): NodeKey | null {
  return t.location.kind === "topic" ? topicKey(t.location.project, t.location.topic) : null;
}

export function panelWidth(canvasW: number): number {
  return Math.min(340, Math.round(canvasW * 0.45));
}

function MapRow({
  row,
  snap,
  topic,
  task,
  project,
  openCount,
  links: lc,
  attention,
  focused,
  lit,
  expanded,
  onClick,
  onFocus,
  onPress,
}: {
  row: LayoutRow;
  snap: { topics: Topic[] };
  topic?: Topic;
  task?: Task;
  project?: Project;
  openCount: number;
  links?: { links: number; waits: boolean };
  attention: boolean;
  focused: boolean;
  lit: boolean;
  expanded: boolean;
  onClick: () => void;
  onFocus: () => void;
  onPress: () => void;
}) {
  const style = { left: row.rect.x, top: row.rect.y, width: row.rect.w, height: row.rect.h };
  const cls = `map-row map-${row.kind}${focused ? " is-focus" : ""}${lit ? "" : " dim"}`;
  if (row.kind === "project") {
    const n = snap.topics.filter((t) => t.project === row.project && t.status !== "done").length;
    return (
      <button type="button" data-key={row.key} className={cls} style={style} onClick={onClick} onFocus={onFocus} onPointerDown={onPress} aria-label={`Projekt ${project?.name ?? row.project}${attention ? ", wymaga uwagi" : ""}`}>
        <span className="map-dot" style={project?.color ? { background: project.color } : undefined} aria-hidden />
        <span className="map-name">{project?.name ?? row.project}</span>
        <span className="map-meta">{plural(n, "temat", "tematy", "tematów").replace(/^/, `${n} `)}</span>
        {attention && <span className="map-attn" title="Wymaga uwagi" aria-hidden />}
      </button>
    );
  }
  if (row.kind === "more") {
    return (
      <button type="button" data-key={row.key} className={cls} style={style} onClick={onClick}>
        więcej w widoku projektu →
      </button>
    );
  }
  if (row.kind === "task") {
    return (
      <button type="button" data-key={row.key} className={cls} style={style} onClick={onClick} onFocus={onFocus} onPointerDown={onPress} aria-label={`Zadanie ${task?.title ?? ""}, ${task ? TASK_STATUS_LABEL[task.status].toLowerCase() : ""}${lc?.links ? `, ${lc.links} połączeń` : ""}`}>
        <span className={`map-task-mark st-${task?.status ?? "todo"}`} aria-hidden />
        <span className="map-label" title={task?.title}>
          {task?.title}
        </span>
        {lc?.waits && <Hourglass size={11} className="map-waits" aria-hidden />}
        {!!lc?.links && <span className="map-links">↔{lc.links}</span>}
      </button>
    );
  }
  const t = topic!;
  const label = [
    `Temat ${t.title}`,
    statusWord(t.status),
    openCount ? `${openCount} ${plural(openCount, "otwarte zadanie", "otwarte zadania", "otwartych zadań")}` : null,
    lc?.links ? `${lc.links} ${plural(lc.links, "połączenie", "połączenia", "połączeń")}` : null,
    lc?.waits ? "na coś czeka" : null,
    attention ? "wymaga uwagi" : null,
  ]
    .filter(Boolean)
    .join(", ");
  return (
    <button
      type="button"
      data-key={row.key}
      className={cls}
      style={style}
      onClick={onClick}
      onFocus={onFocus}
      onPointerDown={onPress}
      aria-label={label}
      aria-expanded={expanded}
      aria-current={focused || undefined}
      data-status={t.status}
    >
      <span className={`badge topic-${t.status}`} aria-hidden>
        <span className="badge-mark" />
      </span>
      <span className="map-label" title={t.title}>
        {t.title}
      </span>
      <span className="map-meta">{statusWord(t.status)}</span>
      {openCount > 0 && <span className="map-meta map-count">{openCount}</span>}
      {lc?.waits && <Hourglass size={12} className="map-waits" aria-hidden />}
      {!!lc?.links && <span className="map-links">↔{lc.links}</span>}
      {attention && <span className="map-attn" title="Wymaga uwagi" aria-hidden />}
    </button>
  );
}

const KIND_LABEL = { waits_out: "Czeka na", waits_in: "Czeka na ten temat", related: "Powiązane" } as const;

function MapPanel({
  nodeKey,
  topic,
  task,
  graph,
  topicByKey,
  taskByKey,
  drawn,
  onClose,
  onFocus,
  onOpen,
  notify,
}: {
  nodeKey: NodeKey;
  topic?: Topic;
  task?: Task;
  graph: Graph;
  topicByKey: Map<NodeKey, Topic>;
  taskByKey: Map<NodeKey, Task>;
  drawn: number;
  onClose: () => void;
  onFocus: (k: NodeKey) => void;
  onOpen: () => void;
  notify: (s: string) => void;
}) {
  const view = useView();
  const { run } = useVault();
  const copy = useCopyContext();
  const [connect, setConnect] = useState(false);
  const near = neighbours(graph, nodeKey);
  const dangling = graph.dangling.filter((d) => d.from === nodeKey);
  const { chain, cycle } = waitChain(graph, nodeKey, 4);
  const projectName = (slug: string) => view.snapshot.projects.find((p) => p.slug === slug)?.name ?? slug;
  const lastEntry = (t: Topic) => journalOf(view.snapshot, t.project, t.slug)[0]?.at ?? null;
  const nodeLabel = (k: NodeKey) => {
    const t = topicByKey.get(k);
    if (t) return { title: t.title, sub: `${projectName(t.project)} · ${statusWord(t.status)}`, sentence: firstSentence(t.summary.split("\n").filter((l) => !/^(Czeka na|Powiązane):/.test(l.trim())).join(" ")) };
    const k2 = taskByKey.get(k);
    if (k2) return { title: k2.title, sub: `zadanie · ${TASK_STATUS_LABEL[k2.status].toLowerCase()}`, sentence: "" };
    return { title: k, sub: "", sentence: "" };
  };
  const remove = async (n: (typeof near)[number]) => {
    // The line lives in the topic the edge starts from.
    const src = topicByKey.get(n.edge.from);
    if (!src) return;
    const to = n.edge.to.startsWith("t:") ? { kind: "topic" as const, project: n.edge.to.slice(2).split("/")[0]!, slug: n.edge.to.slice(2).split("/")[1]! } : { kind: "task" as const, id: n.edge.to.slice(2) };
    const next = removeRefLine(src.summary, n.edge.kind, to, src.project);
    if (next === null) return notify("To połączenie jest w treści opisu: zmień je w edycji tematu.");
    const ok = await run((a) => a.updateTopic(src.project, src.slug, { summary: next }));
    if (ok) notify("Usunięto połączenie.");
  };
  const groups: [keyof typeof KIND_LABEL, typeof near][] = [
    ["waits_out", near.filter((n) => n.edge.kind === "waits" && n.outgoing)],
    ["waits_in", near.filter((n) => n.edge.kind === "waits" && !n.outgoing)],
    ["related", near.filter((n) => n.edge.kind === "related")],
  ];
  const title = topic?.title ?? task?.title ?? "";
  return (
    <aside className="map-panel" aria-label={`Szczegóły: ${title}`} onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), onClose())}>
      <div className="map-panel-head">
        <h2 className="map-panel-title">{title}</h2>
        <button type="button" className="icon-btn" aria-label="Zamknij panel" onClick={onClose}>
          ×
        </button>
      </div>
      {topic && (
        <>
          <p className="map-panel-sub">
            {projectName(topic.project)} · <TopicStatusPill project={{ slug: topic.project }} topic={topic} />
            {lastEntry(topic) && <span className="map-panel-age"> · ostatni wpis {formatAge(lastEntry(topic)!, view.today)}</span>}
          </p>
          {topic.summary && <p className="map-panel-summary">{topic.summary}</p>}
          {chain.length > 0 && (
            <div className="map-chain" aria-label="Łańcuch oczekiwania">
              <span className="map-chain-label">Czeka na</span>
              {chain.map((k, i) => {
                const t = topicByKey.get(k);
                const tk = taskByKey.get(k);
                const at = t ? lastEntry(t) : null;
                return (
                  <span key={k} className="map-chain-step">
                    {i > 0 && <span aria-hidden> → </span>}
                    <button type="button" className="map-link" onClick={() => onFocus(k)}>
                      {t?.title ?? tk?.title ?? k}
                    </button>
                    <span className="map-chain-meta">
                      {" "}
                      ({t ? statusWord(t.status) : tk ? TASK_STATUS_LABEL[tk.status].toLowerCase() : "?"}
                      {t ? `, ${at ? `wpis ${formatAge(at, view.today)}` : "bez wpisów"}` : ""})
                    </span>
                  </span>
                );
              })}
              {cycle && <span className="map-chain-meta"> → pętla (przerwana)</span>}
            </div>
          )}
          <OpenTasks project={topic.project} slug={topic.slug} onFocus={onFocus} />
        </>
      )}
      {task && (
        <>
          <p className="map-panel-sub">
            {task.location.kind === "inbox" ? "Skrzynka" : projectName(task.location.project)}
            {task.location.kind === "topic" && ` / ${topicByKey.get(topicKey(task.location.project, task.location.topic))?.title ?? task.location.topic}`} · {TASK_STATUS_LABEL[task.status].toLowerCase()}
          </p>
          {task.notes && <p className="map-panel-summary">{task.notes}</p>}
        </>
      )}

      <h3 className="map-panel-h">
        Połączenia {near.length > 0 && <span className="count">{near.length}</span>}
        {near.length > drawn && near.length > 0 && <span className="map-panel-note"> · na mapie {drawn}, +{near.length - drawn} tylko tutaj</span>}
      </h3>
      {near.length === 0 && <p className="map-panel-empty">Brak. {topic ? "Dodaj przez „Połącz z…” albo wpisz [[projekt/temat]] w opisie stanu." : "Wpisz ^id albo [[projekt/temat]] w notatkach zadania."}</p>}
      {groups.map(
        ([kind, items]) =>
          items.length > 0 && (
            <div key={kind} className="map-conn-group">
              <h4 className="map-conn-head">{KIND_LABEL[kind]}</h4>
              <ul className="map-conn-list">
                {items.map((n) => {
                  const l = nodeLabel(n.other);
                  const srcTopic = topicByKey.get(n.edge.from);
                  return (
                    <li key={`${n.other}-${n.edge.kind}-${n.outgoing}`} className="map-conn">
                      <button type="button" className="map-link" onClick={() => onFocus(n.other)}>
                        {l.title}
                      </button>
                      <span className="map-conn-sub">{l.sub}</span>
                      {l.sentence && <span className="map-conn-sentence">{l.sentence}</span>}
                      {n.edge.standalone && srcTopic ? (
                        <button type="button" className="map-link map-remove" onClick={() => void remove(n)} title={`Usuwa linię z opisu tematu „${srcTopic.title}”`}>
                          Usuń
                        </button>
                      ) : (
                        <span className="map-conn-where">{n.edge.source === "notes" ? "w notatce" : "w opisie"}</span>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          ),
      )}
      {dangling.length > 0 && (
        <div className="map-warn" role="note">
          {dangling.map((d, i) => (
            <p key={i}>Odwołanie donikąd: {d.text}</p>
          ))}
        </div>
      )}
      {topic && connect && <ConnectForm topic={topic} onDone={() => setConnect(false)} />}
      <div className="map-panel-actions">
        <Button variant="ghost" onClick={onOpen}>
          Otwórz w projekcie
        </Button>
        {topic && (
          <Button variant="quiet" onClick={() => void copy(topic.project, topic)}>
            Kopiuj kontekst
          </Button>
        )}
        {topic && !connect && (
          <Button variant="quiet" onClick={() => setConnect(true)}>
            Połącz z…
          </Button>
        )}
      </div>
    </aside>
  );
}

function OpenTasks({ project, slug, onFocus }: { project: string; slug: string; onFocus: (k: NodeKey) => void }) {
  const view = useView();
  const open = view.snapshot.tasks.filter((t) => t.status !== "done" && t.location.kind === "topic" && t.location.project === project && t.location.topic === slug);
  if (!open.length) return null;
  return (
    <>
      <h3 className="map-panel-h">
        Otwarte zadania <span className="count">{open.length}</span>
      </h3>
      <ul className="map-panel-tasks">
        {open.slice(0, 10).map((t) => (
          <li key={taskKey(t)}>
            <button type="button" className="map-link" onClick={() => onFocus(taskKey(t))}>
              {t.title}
            </button>{" "}
            <span className="map-conn-sub">{TASK_STATUS_LABEL[t.status].toLowerCase()}</span>
          </li>
        ))}
        {open.length > 10 && <li className="map-conn-sub">+{open.length - 10}</li>}
      </ul>
    </>
  );
}

/** "Połącz z…": appends one standalone line to this topic's summary (the rest stays as it is). */
function ConnectForm({ topic, onDone }: { topic: Topic; onDone: () => void }) {
  const view = useView();
  const { run, notify } = useVault();
  const [target, setTarget] = useState("");
  const [kind, setKind] = useState<EdgeKind>("related");
  const [busy, setBusy] = useState(false);
  const active = view.snapshot.projects.filter((p) => p.state === "active");
  const candidates = active.map((p) => ({ p, topics: view.snapshot.topics.filter((t) => t.project === p.slug && t.status !== "done" && !(t.project === topic.project && t.slug === topic.slug)) })).filter((g) => g.topics.length);
  async function save() {
    const [p, s] = target.split("/") as [string, string];
    const t = view.snapshot.topics.find((x) => x.project === p && x.slug === s);
    if (!t) return;
    setBusy(true);
    try {
      const summary = appendRefLine(topic.summary, kind, t, topic.project);
      const ok = await run((a) => a.updateTopic(topic.project, topic.slug, { summary }));
      if (ok) {
        notify(kind === "waits" ? `„${topic.title}” czeka teraz na „${t.title}”.` : `Połączono z „${t.title}”.`);
        onDone();
      }
    } catch (e) {
      notify(describeError(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="map-connect"
      aria-label="Połącz z tematem"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), onDone())}
    >
      <label className="map-connect-label">
        Temat
        <select className="field select" value={target} autoFocus onChange={(e) => setTarget(e.target.value)}>
          <option value="">Wybierz temat…</option>
          {candidates.map((g) => (
            <optgroup key={g.p.slug} label={g.p.name}>
              {g.topics.map((t) => (
                <option key={t.slug} value={`${t.project}/${t.slug}`}>
                  {t.title}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      </label>
      <Segmented<EdgeKind>
        label="Rodzaj połączenia"
        value={kind}
        onChange={setKind}
        options={[
          { value: "related", label: "Powiązane" },
          { value: "waits", label: "Czeka na" },
        ]}
      />
      <p className="map-connect-note">Dopisze jedną linię do opisu stanu: {kind === "waits" ? "Czeka na:" : "Powiązane:"} [[{target || "projekt/temat"}]]</p>
      <div className="map-panel-actions">
        <Button type="submit" variant="ghost" disabled={!target || busy}>
          {busy ? "Zapisuję…" : "Połącz"}
        </Button>
        <Button variant="quiet" onClick={onDone}>
          Anuluj
        </Button>
      </div>
    </form>
  );
}

import { t as tr } from "../i18n";
import { askAboutTopic } from "../store/session";
// „Mapa": projects as islands on a pannable canvas, their topics as rows
// along a spine, open tasks on expanding, and the connections between topics
// and tasks written as references in summaries and notes (lib/refs.ts).
// Rows are real buttons in document order over an SVG layer (spines and the
// focused node's curves). Layout: lib/mapLayout.ts (pure; zoom only scales).

import { Hourglass, Maximize, Minus, Plus } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import type { Project, Task, Topic, TopicStatus } from "../api";
import { useCopyContext } from "../components/TopicItem";
import { Button, ProjectName, Segmented } from "../components/ui";
import { DetailPanel } from "../components/DetailPanel";
import { resolveDetail, taskTarget, topicTarget } from "../lib/details";
import { lockInert } from "../lib/inert";
import { formatAge } from "../lib/dates";
import { describeError } from "../lib/errors";
import { journalOf, partitionProjects } from "../lib/group";
import { TASK_STATUS_LABEL, TOPIC_STATUS_LABEL } from "../lib/labels";
import { fitView, layoutMap, routeCurve, type LayoutRow, type MapLayout } from "../lib/mapLayout";
import { appendRefLine, buildGraph, counts, neighbours, referenceProse, removeRefLine, taskKey, topicKey, waitChain, type EdgeKind, type Graph, type NodeKey } from "../lib/refs";
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
const PRESENTATION_PREF = "montybrain.map.presentation";
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
  const [presentation, setPresentation] = useState<"map" | "list">(() => readPref(PRESENTATION_PREF) === "list" ? "list" : "map");
  const choosePresentation = (next: "map" | "list") => {
    setPresentation(next);
    writePref(PRESENTATION_PREF, next);
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
  const [panelOverlay, setPanelOverlay] = useState(false);
  useEffect(() => {
    const content = canvas.current?.closest(".main-scroll");
    if (!content) return;
    const observer = new ResizeObserver(([entry]) => setPanelOverlay(entry!.contentRect.width < 1000));
    observer.observe(content);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!focus || !panelOverlay || !canvas.current) return;
    const controls = [...(canvas.current.closest(".map-view")?.children ?? [])].filter((el) => !el.classList.contains("map-workspace")) as HTMLElement[];
    const sidebar = document.querySelector<HTMLElement>(".sidebar");
    return lockInert([canvas.current, ...controls, ...(sidebar ? [sidebar] : [])]);
  }, [focus, panelOverlay]);
  const closePanel = () => {
    setFocus(null);
    // Focusing a map row selects it again; restore focus to the canvas.
    requestAnimationFrame(() => canvas.current?.focus({ preventScroll: true }));
  };
  const [vp, setVp] = useState<{ x: number; y: number; z: number } | null>(() => {
    const v = readJson<{ x: number; y: number; z: number } | null>(VIEW_PREF, null);
    return v && Number.isFinite(v.z) ? v : null;
  });
  const [fitted, setFitted] = useState(() => !vp || readPref(`${VIEW_PREF}.fitted`) === "true");
  const fittedRef = useRef(fitted);
  fittedRef.current = fitted;
  const focusRef = useRef(focus);
  focusRef.current = focus;
  // The latest view, for effects that run before a re-render (fit, then reveal).
  const vRef = useRef(vp);
  vRef.current = vp ?? vRef.current;
  const winRef = useRef({ w: window.innerWidth, h: window.innerHeight });
  const setView = (v: { x: number; y: number; z: number }, autoFit = false) => {
    setFitted(autoFit);
    writePref(`${VIEW_PREF}.fitted`, String(autoFit));
    vRef.current = v;
    setVp(v);
    writePref(VIEW_PREF, JSON.stringify(v));
    writePref(`${VIEW_PREF}.layout`, layoutKey);
    if (canvas.current) writePref(`${VIEW_PREF}.size`, JSON.stringify({ w: canvas.current.clientWidth, h: canvas.current.clientHeight }));
    writePref(`${VIEW_PREF}.window`, JSON.stringify({ w: window.innerWidth, h: window.innerHeight }));
  };
  const fit = () => {
    const el = canvas.current;
    if (el && presentation === "map") setView(fitView(layout, { w: el.clientWidth, h: el.clientHeight }), true);
  };
  useLayoutEffect(() => {
    if (presentation !== "map") return;
    // Preserve manual camera framing across view switches and layout changes;
    // only auto-fit on first mount or when an auto-fitted map's window/layout changed.
    const savedWindow = readJson<{ w: number; h: number } | null>(`${VIEW_PREF}.window`, null);
    const resized = savedWindow && (savedWindow.w !== window.innerWidth || savedWindow.h !== window.innerHeight);
    if (!vp || (fittedRef.current && (resized || readPref(`${VIEW_PREF}.layout`) !== layoutKey))) fit();
  }, [layoutKey, presentation]);
  const v = vp ?? { x: 0, y: 0, z: 1 };
  const zoomAt = (factor: number, cx: number, cy: number) => {
    const cur = vRef.current ?? { x: 0, y: 0, z: 1 };
    const z = Math.min(2, Math.max(0.2, cur.z * factor));
    const k = z / cur.z;
    setView({ z, x: cx - (cx - cur.x) * k, y: cy - (cy - cur.y) * k });
  };
  // Wheel: pan; with Ctrl (pinch on a trackpad): zoom around the pointer.
  useEffect(() => {
    const el = canvas.current;
    if (!el || presentation !== "map") return;
    const onWheel = (e: WheelEvent) => {
      if ((e.target as HTMLElement).closest(".map-panel")) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) zoomAt(Math.exp(-e.deltaY / 300), e.clientX - r.left, e.clientY - r.top);
      else {
        const cur = vRef.current ?? { x: 0, y: 0, z: 1 };
        setView({ ...cur, x: cur.x - e.deltaX, y: cur.y - e.deltaY });
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [presentation]);
  const drag = useRef<{ x: number; y: number; vx: number; vy: number; z: number; moved: boolean } | null>(null);
  const onPointerDown = (e: ReactPointerEvent) => {
    if ((e.target as HTMLElement).closest("button, a, input, select, .map-panel")) return;
    const cur = vRef.current ?? { x: 0, y: 0, z: 1 };
    drag.current = { x: e.clientX, y: e.clientY, vx: cur.x, vy: cur.y, z: cur.z, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: ReactPointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
    const next = { z: d.z, x: d.vx + dx, y: d.vy + dy };
    vRef.current = next;
    setVp(next);
  };
  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.moved && vRef.current) {
      setView(vRef.current);
    }
    else if (d && !d.moved) setFocus(null);
  };

  /** Brings a row into view without shrinking an already-readable map scale. */
  const reveal = (key: NodeKey, closer = false) => {
    const r = rowByKey.get(key) ?? (taskByKey.get(key) ? rowByKey.get(parentTopic(taskByKey.get(key)!) ?? "") : undefined);
    const el = canvas.current;
    if (!r || !el) return;
    if (presentation === "list") {
      el.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`)?.scrollIntoView({ block: "nearest" });
      return;
    }
    const cur = vRef.current ?? v;
    const w = el.clientWidth;
    // Too small to read on large maps or ?focus= deep-links: zoom up smoothly without shrinking normal maps.
    let z = cur.z;
    if (closer && z < 0.8) z = 0.85;
    else if (fittedRef.current && z < 0.72) z = 0.78;
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
  }, [focus, presentation]);
  useLayoutEffect(() => {
    const el = canvas.current;
    if (!el || presentation !== "map") return;
    // ResizeObserver fires once on observation, and also when MapPanel opens/closes.
    // Only re-fit when the actual browser window resizes while in auto-fit mode;
    // panel open/close keeps zoom intact and pans the focused node into view if needed.
    let width = el.clientWidth;
    let height = el.clientHeight;
    const ro = new ResizeObserver(() => {
      if (width === el.clientWidth && height === el.clientHeight) return;
      width = el.clientWidth;
      height = el.clientHeight;
      const winResized = winRef.current.w !== window.innerWidth || winRef.current.h !== window.innerHeight;
      winRef.current = { w: window.innerWidth, h: window.innerHeight };
      if (winResized && fittedRef.current) fit();
      else if (focusRef.current) reveal(focusRef.current);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [layoutKey, presentation]);
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
    : graph.edges.filter((edge) => edge.kind === "waits")
        .map((edge) => ({ n: { edge, other: edge.to, outgoing: true }, a: anchor(edge.from), b: anchor(edge.to) }))
        .filter((x) => x.a && x.b && x.a.key !== x.b.key)
        .slice(0, MAX_CURVES);
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
      else focusRow(focus);
    } else if (e.key === "Escape") {
      setFocus(null);
    } else if (e.key === "0") {
      e.preventDefault();
      fit();
    } else if (presentation === "map" && (e.key === "+" || e.key === "=")) {
      zoomAt(1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2);
    } else if (presentation === "map" && e.key === "-") {
      zoomAt(1 / 1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2);
    }
  };

  // A click focuses the button before its click event: that focus must not count as the first click.
  const pressed = useRef<string | null>(null);
  const onRowClick = (r: LayoutRow) => {
    pressed.current = null;
    if (r.kind === "more" && r.topic) return setFocus(topicKey(r.project, r.topic));
    if (r.kind === "project") return setFocus(focus === r.key ? null : r.key);
    // A second click on a focused topic expands (or collapses) it.
    if (focus === r.key) toggleExpand(r);
    else setFocus(r.key);
  };

  const projectOf = (slug: string) => snap.projects.find((p) => p.slug === slug);
  const empty = layout.rows.length === 0;

  const renderRow = (r: LayoutRow) => (
    <MapRow
      key={r.key}
      row={r}
      list={presentation === "list"}
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
  );

  return (
    <div className="view map-view">
      <div className="map-toolbar" role="toolbar" aria-label={tr("Mapa: widok i filtry")}>
        <h1 className="map-title">{tr("Mapa")}</h1>
        <Segmented<"map" | "list"> label={tr("Sposób przeglądania")} value={presentation} onChange={choosePresentation} options={[{ value: "map", label: tr("Mapa") }, { value: "list", label: tr("Lista") }]} />
        <select className="field select map-select" style={filters.project ? { color: projectOf(filters.project)?.color ?? undefined } : undefined} aria-label={tr("Projekt")} value={filters.project ?? ""} onChange={(e) => setFilters({ ...filters, project: e.target.value || null })}>
          <option value="">{tr("Wszystkie projekty")}</option>
          {[...visible, ...archived]
            .filter((p) => filters.done || p.state === "active")
            .map((p) => (
              <option key={p.slug} value={p.slug} style={p.color ? { color: p.color } : undefined}>
                {p.name}
              </option>
            ))}
        </select>
        <label className="map-check">
          <input type="checkbox" checked={filters.attention} onChange={(e) => setFilters({ ...filters, attention: e.target.checked })} />  {tr("Wyróżnij wymagające uwagi")}</label>
        <label className="map-check">
          <input type="checkbox" checked={filters.done} onChange={(e) => setFilters({ ...filters, done: e.target.checked })} />  {tr("Pokaż zakończone")}</label>
        {presentation === "map" && <span className="map-zoom">
          <span className="map-scale" aria-label={tr("Powiększenie")}>{Math.round(v.z * 100)}%</span>
          <button type="button" className="icon-btn" aria-label={tr("Pomniejsz")} title={tr("Pomniejsz (−)")} onClick={() => zoomAt(1 / 1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2)}>
            <Minus size={14} />
          </button>
          <button type="button" className="icon-btn" aria-label={tr("Powiększ")} title={tr("Powiększ (+)")} onClick={() => zoomAt(1.2, (canvas.current?.clientWidth ?? 0) / 2, (canvas.current?.clientHeight ?? 0) / 2)}>
            <Plus size={14} />
          </button>
          <button type="button" className="btn btn-quiet map-fit" title={tr("Dopasuj widok (0)")} onClick={fit}>
            <Maximize size={13} />  {tr("Dopasuj")}</button>
        </span>}
      </div>

      <p className="map-help" id="map-help">{tr("Wybierz temat, aby zobaczyć połączenia · → rozwiń zadania · liczba = otwarte zadania · ↔ = połączenia ·")} <Hourglass size={11} aria-hidden />  {tr("czeka na ·")} <span className="map-attn" aria-hidden />  {tr("wymaga uwagi")}</p>
      {presentation === "map" && <p className="map-readability">{tr("Przegląd mapy.")} <button type="button" className="map-inline-link" onClick={() => choosePresentation("list")}>{tr("Czytaj pełne nazwy w Liście")}</button>  {tr("lub powiększ widok.")}</p>}
      <div className="map-workspace">
        <div
          ref={canvas}
          className={`${presentation === "list" ? "map-list" : "map-canvas"}${focus ? " focused" : ""}${filters.attention ? " attention" : ""}`}
          tabIndex={0}
          aria-describedby="map-help"
          onKeyDown={onKeyDown}
          onPointerDown={presentation === "map" ? onPointerDown : undefined}
          onPointerMove={presentation === "map" ? onPointerMove : undefined}
          onPointerUp={presentation === "map" ? onPointerUp : undefined}
          aria-label={tr("Mapa projektów i tematów. Strzałki: wiersze; Enter: otwórz; strzałka w prawo: zadania; Esc: koniec wyboru; 0: dopasuj widok.")}
          role={presentation === "map" ? "application" : "region"}
        >
          {empty ? (
            <p className="empty-text map-empty">{tr("Brak aktywnych projektów z tematami. Włącz „Pokaż zakończone” albo dodaj temat w widoku projektu.")}</p>
          ) : presentation === "list" ? (
            layout.islands.map((island) => (
              <section key={island.project} className="map-list-group" aria-label={projectOf(island.project)?.name ?? island.project}>
                {layout.rows.filter((r) => r.project === island.project).map(renderRow)}
              </section>
            ))
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
              {layout.rows.map(renderRow)}
            </div>
          )}
        </div>
        {(focusTopic || focusTask) && panelOverlay && <div className="map-backdrop" aria-hidden onClick={closePanel} />}
        {(focusTopic || focusTask) && (
          <MapPanel
            key={focus!}
            overlay={panelOverlay}
            nodeKey={focus!}
            topic={focusTopic}
            task={focusTask}
            graph={graph}
            topicByKey={topicByKey}
            taskByKey={taskByKey}
            drawn={drawn.length}
            onClose={closePanel}
            onFocus={(k) => focusRow(k)}
            onShowOnMap={presentation === "list" ? () => choosePresentation("map") : undefined}
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
  return Math.min(440, canvasW <= 600 ? canvasW : Math.round(canvasW * 0.52));
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
  list = false,
}: {
  row: LayoutRow;
  list?: boolean;
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
  const style = list ? undefined : { left: row.rect.x, top: row.rect.y, width: row.rect.w, height: row.rect.h };
  const cls = `map-row map-${row.kind}${focused ? " is-focus" : ""}${lit ? "" : " dim"}`;
  if (row.kind === "project") {
    const n = snap.topics.filter((t) => t.project === row.project && t.status !== "done").length;
    return (
      <button type="button" data-key={row.key} className={cls} style={style} onClick={onClick} onFocus={onFocus} onPointerDown={onPress} aria-label={tr("Projekt {0}{1}", project?.name ?? row.project, attention ? tr(", wymaga uwagi") : "")}>
        <ProjectName name={project?.name ?? row.project} color={project?.color} className="map-name" />
        <span className="map-meta">{plural(n, tr("temat"), tr("tematy"), tr("tematów")).replace(/^/, `${n} `)}</span>
        {attention && <span className="map-attn" title={tr("Wymaga uwagi")} aria-hidden />}
      </button>
    );
  }
  if (row.kind === "more") {
    return (
      <button type="button" data-key={row.key} className={cls} style={style} onClick={onClick}>
        {tr("wszystkie zadania w panelu →")}</button>
    );
  }
  if (row.kind === "task") {
    return (
      <button type="button" data-key={row.key} className={cls} style={style} onClick={onClick} onFocus={onFocus} onPointerDown={onPress} aria-label={tr("Zadanie {0}, {1}{2}", task?.title ?? "", task ? TASK_STATUS_LABEL[task.status].toLowerCase() : "", lc?.links ? tr(", {0} połączeń", lc.links) : "")}>
        <span className={`map-task-mark st-${task?.status ?? "todo"}`} aria-hidden />
        <span className="map-label" title={task?.title}>
          {task?.title}
        </span>{" "}
        {lc?.waits && <Hourglass size={11} className="map-waits" aria-hidden />}
        {!!lc?.links && <span className="map-links" title={`${lc.links} ${plural(lc.links, tr("połączenie"), tr("połączenia"), tr("połączeń"))}`}>↔{lc.links}</span>}
      </button>
    );
  }
  const t = topic!;
  const label = [
    tr("Temat {0}", t.title),
    statusWord(t.status),
    openCount ? `${openCount} ${plural(openCount, tr("otwarte zadanie"), tr("otwarte zadania"), tr("otwartych zadań"))}` : null,
    lc?.links ? `${lc.links} ${plural(lc.links, tr("połączenie"), tr("połączenia"), tr("połączeń"))}` : null,
    lc?.waits ? tr("na coś czeka") : null,
    attention ? tr("wymaga uwagi") : null,
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
      </span>{" "}
      {list ? (
        <span className="map-topic-meta">
          <span className="map-meta">{statusWord(t.status)}</span>
          <span className="map-meta map-count" title={tr("{0} otwartych zadań", openCount)}>{openCount}</span>
          <span className="map-wait-slot" aria-hidden>{lc?.waits ? <Hourglass size={12} /> : null}</span>
          <span className="map-links" title={lc?.links ? tr("{0} połączeń", lc.links) : undefined}>{lc?.links ? `↔${lc.links}` : ""}</span>
          <span className={`map-attn${attention ? "" : " is-empty"}`} title={attention ? tr("Wymaga uwagi") : undefined} aria-hidden />
        </span>
      ) : <>
      <span className="map-meta">{statusWord(t.status)}</span>
      {openCount > 0 && <>{" "}<span className="map-meta map-count" title={`${openCount} ${plural(openCount, tr("otwarte zadanie"), tr("otwarte zadania"), tr("otwartych zadań"))}`}>{openCount}</span></>}
      {lc?.waits && <Hourglass size={12} className="map-waits" aria-hidden />}
      {!!lc?.links && <>{" "}<span className="map-links" title={`${lc.links} ${plural(lc.links, tr("połączenie"), tr("połączenia"), tr("połączeń"))}`}>↔{lc.links}</span></>}
      {attention && <span className="map-attn" title={tr("Wymaga uwagi")} aria-hidden />}
      </>}
    </button>
  );
}

const KIND_LABEL = { get waits_out() { return tr("Czeka na"); }, get waits_in() { return tr("Czeka na ten temat"); }, get related() { return tr("Powiązane"); } } as const;

function MapPanel({
  overlay,
  nodeKey,
  topic,
  task,
  graph,
  topicByKey,
  taskByKey,
  drawn,
  onClose,
  onFocus,
  onShowOnMap,
  notify,
}: {
  overlay: boolean;
  nodeKey: NodeKey;
  topic?: Topic;
  task?: Task;
  graph: Graph;
  topicByKey: Map<NodeKey, Topic>;
  taskByKey: Map<NodeKey, Task>;
  drawn: number;
  onClose: () => void;
  onFocus: (k: NodeKey) => void;
  onShowOnMap?: () => void;
  notify: (s: string) => void;
}) {
  const view = useView();
  const { run } = useVault();
  const copy = useCopyContext();
  const [connect, setConnect] = useState(false);
  const near = neighbours(graph, nodeKey);
  const dangling = graph.dangling.filter((d) => d.from === nodeKey);
  const { chain, cycle } = waitChain(graph, nodeKey, 4);
  const lastEntry = (t: Topic) => journalOf(view.snapshot, t.project, t.slug)[0]?.at ?? null;
  const nodeLabel = (k: NodeKey) => {
    const t = topicByKey.get(k);
    if (t) return { title: t.title, project: view.snapshot.projects.find((p) => p.slug === t.project), sub: statusWord(t.status), sentence: firstSentence(t.summary.split("\n").filter((l) => !/^(Czeka na|Powiązane):/.test(l.trim())).join(" ")) };
    const k2 = taskByKey.get(k);
    if (k2) return { title: k2.title, project: undefined, sub: tr("zadanie · {0}", TASK_STATUS_LABEL[k2.status].toLowerCase()), sentence: "" };
    return { title: k, project: undefined, sub: "", sentence: "" };
  };
  const remove = async (n: (typeof near)[number]) => {
    // The line lives in the topic the edge starts from.
    const src = topicByKey.get(n.edge.from);
    if (!src) return;
    const to = n.edge.to.startsWith("t:") ? { kind: "topic" as const, project: n.edge.to.slice(2).split("/")[0]!, slug: n.edge.to.slice(2).split("/")[1]! } : { kind: "task" as const, id: n.edge.to.slice(2) };
    const next = removeRefLine(src.summary, n.edge.kind, to, src.project);
    if (next === null) return notify(tr("To połączenie jest w treści opisu: zmień je w edycji tematu."));
    const ok = await run((a) => a.updateTopic(src.project, src.slug, { summary: next }));
    if (ok) notify(tr("Usunięto połączenie."));
  };
  const groups: [keyof typeof KIND_LABEL, typeof near][] = [
    ["waits_out", near.filter((n) => n.edge.kind === "waits" && n.outgoing)],
    ["waits_in", near.filter((n) => n.edge.kind === "waits" && !n.outgoing)],
    ["related", near.filter((n) => n.edge.kind === "related")],
  ];
  if (!topic && !task) return null;
  return (
    <DetailPanel
      target={topic ? topicTarget(topic) : taskTarget(task!)}
      className="map-panel"
      overlay={overlay}
      summaryText={topic ? referenceProse(topic.summary, topic.project) : undefined}
      onClose={onClose}
      onSelect={(target) => {
        const item = resolveDetail(view.snapshot, target);
        if (target.kind === "task" && item.task) onFocus(taskKey(item.task));
        else if (item.topic) onFocus(topicKey(item.topic.project, item.topic.slug));
      }}
    >
      {topic && (
        <>
          {(chain.length > 1 || cycle) && (
            <div className="map-chain" aria-label={tr("Łańcuch oczekiwania")}>
              <span className="map-chain-label">{tr("Czeka na")}</span>{" "}
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
                      {t ? `, ${at ? tr("wpis {0}", formatAge(at, view.today)) : tr("bez wpisów")}` : ""})
                    </span>
                  </span>
                );
              })}
              {cycle && <span className="map-chain-meta">  {tr("→ pętla (przerwana)")}</span>}
            </div>
          )}
        </>
      )}
      <h3 className="map-panel-h">
        {tr("Połączenia")} {near.length > 0 && <span className="count">{near.length}</span>}
        {near.length > drawn && near.length > 0 && <span className="map-panel-note">  {tr("· na mapie")} {drawn}, +{near.length - drawn}  {tr("tylko tutaj")}</span>}
      </h3>
      {near.length === 0 && <p className="map-panel-empty">{tr("Brak.")} {topic ? tr("Dodaj przez „Połącz z…” albo wpisz [[projekt/temat]] w opisie stanu.") : tr("Wpisz ^id albo [[projekt/temat]] w notatkach zadania.")}</p>}
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
                      <span className="map-conn-sub">{l.project && <><ProjectName name={l.project.name} color={l.project.color} /> · </>}{l.sub}</span>
                      {l.sentence && <span className="map-conn-sentence">{l.sentence}</span>}
                      {n.edge.standalone && srcTopic ? (
                        <button type="button" className="map-link map-remove" onClick={() => void remove(n)} title={tr("Usuwa linię z opisu tematu „{0}”", srcTopic.title)}>
                          {tr("Usuń")}</button>
                      ) : (
                        <span className="map-conn-where">{n.edge.source === "notes" ? tr("w notatce") : tr("w opisie")}</span>
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
            <p key={i} title={tr("Odwołanie w pliku: {0}", d.text)}>{tr("Nie znaleziono")} {d.text.startsWith("^") ? tr("zadania") : tr("tematu")}: {d.text.replace(/^\[\[|\]\]$/g, "").replaceAll("-", " ")}</p>
          ))}
        </div>
      )}
      {topic && connect && <ConnectForm topic={topic} onDone={() => setConnect(false)} />}
      <div className="map-panel-actions">
        {onShowOnMap && <Button variant="quiet" onClick={onShowOnMap}>{tr("Pokaż na mapie")}</Button>}
        {topic && (
          <Button variant="quiet" onClick={() => void copy(topic.project, topic)}>
            {tr("Kopiuj kontekst")}</Button>
        )}
        {topic && <Button variant="quiet" onClick={() => askAboutTopic(topic)}>{tr("Zapytaj agenta…")}</Button>}
        {topic && !connect && (
          <Button variant="quiet" onClick={() => setConnect(true)}>
            {tr("Połącz z…")}</Button>
        )}
      </div>
    </DetailPanel>
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
        notify(kind === "waits" ? tr("„{0}” czeka teraz na „{1}”.", topic.title, t.title) : tr("Połączono z „{0}”.", t.title));
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
      aria-label={tr("Połącz z tematem")}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), onDone())}
    >
      <label className="map-connect-label">
        {tr("Temat")}<select className="field select" value={target} autoFocus onChange={(e) => setTarget(e.target.value)}>
          <option value="">{tr("Wybierz temat…")}</option>
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
        label={tr("Rodzaj połączenia")}
        value={kind}
        onChange={setKind}
        options={[
          { value: "related", label: tr("Powiązane") },
          { value: "waits", label: tr("Czeka na") },
        ]}
      />
      <p className="map-connect-note">{tr("Dopisze jedną linię do opisu stanu:")} {kind === "waits" ? tr("Czeka na:") : tr("Powiązane:")} [[{target || "projekt/temat"}]]</p>
      <div className="map-panel-actions">
        <Button type="submit" variant="ghost" disabled={!target || busy}>
          {busy ? tr("Zapisuję…") : tr("Połącz")}
        </Button>
        <Button variant="quiet" onClick={onDone}>
          {tr("Anuluj")}</Button>
      </div>
    </form>
  );
}

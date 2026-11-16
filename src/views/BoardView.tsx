import { t as tr, getLanguage } from "../i18n";
// „Tablica": kanban over task statuses or topic statuses. Cards are dragged with
// pointer events (no dependency) and can be moved with the keyboard
// (⌥←/⌥→, 1…5 or `m`). Moves are shown immediately and reconciled with the
// reloaded state; a failed move rolls back through the usual error toast.

import { ChevronRight, ArrowDown, MoreHorizontal } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";

import { createPortal } from "react-dom";

import type { Snapshot, Task, TaskLocation, TaskStatus, Topic, TopicStatus } from "../api";
import { LinkChip } from "../components/TaskRow";
import { Menu, type MenuItem } from "../components/Menu";
import { StatusGlyph } from "../components/StatusToggle";
import { TopicStatusBadge, TopicStatusPill } from "../components/TopicItem";
import { Button, Segmented, ProjectName, useStoredFlag } from "../components/ui";
import {
  ALL_PROJECTS,
  DONE_CAP,
  TASK_COLUMNS,
  TOPIC_COLUMNS,
  TOPIC_BOARD_COLUMNS,
  TOPIC_ARCHIVE_DAYS,
  adjacentColumn,
  beginMove,
  capColumn,
  createSerialQueue,
  effectiveScope,
  groupTasks,
  groupTopics,
  moveOverrides,
  settleMove,
  taskLanes,
  topicId,
  topicMoveColumns,
  type BoardMode,
  type TopicColumn,
  type PendingMoves,
} from "../lib/board";
import { formatAge, formatDue, isOverdue } from "../lib/dates";
import { countTasks, partitionProjects, taskContext, topicKey } from "../lib/group";
import { TASK_STATUS_KEY, TASK_STATUS_LABEL, TOPIC_STATUSES, TOPIC_STATUS_LABEL } from "../lib/labels";
import { openCount } from "../lib/topicStatus";
import { taskTarget, topicTarget } from "../lib/details";
import { useOpenDetail } from "../store/details";
import { readPref, writePref } from "../store/ui";
import { useVault, useView } from "../store/vault";
import { plural } from "../lib/text";
import { anchoredScrollTop, applyTaskDrop, taskDrop, type TaskDrop } from "../lib/boardDrop";
import { topicSummary } from "../lib/topicSummary";
import "./board.css";

type Status = TaskStatus | TopicStatus;

interface DropTarget {
  col: string;
  location: TaskLocation | null;
  lane: string | null;
  card: string | null;
}
interface Drag { id: string; from: string; over: DropTarget | null; }
interface CardSpot { y: number; sourceY: number; left: number; }
interface ViewAnchor extends CardSpot { id: string; op: number; settled: boolean; }
interface TopicChange { task: Task; drop: TaskDrop; spot: CardSpot; }


const MODE_PREF = "montybrain.board.mode";
const SCOPE_PREF = "montybrain.board.scope";

export function BoardView() {
  const openDetail = useOpenDetail();
  const view = useView();
  const { notify, run } = useVault();
  const snap = view.snapshot;

  const [mode, setModeState] = useState<BoardMode>(() => (readPref(MODE_PREF) === "topics" ? "topics" : "tasks"));
  const [scopePref, setScopePref] = useState<string>(() => readPref(SCOPE_PREF) ?? ALL_PROJECTS);
  const scope = effectiveScope(snap, scopePref);
  const [taskDoneOpen, setTaskDoneOpen] = useStoredFlag("montybrain.board.done.tasks", true);
  const [topicDoneOpen, setTopicDoneOpen] = useStoredFlag("montybrain.board.done.topics", true);
  const [topicArchiveOpen, setTopicArchiveOpen] = useStoredFlag("montybrain.board.archive.topics", false);
  const doneOpen = mode === "tasks" ? taskDoneOpen : topicDoneOpen;
  const setDoneOpen = mode === "tasks" ? setTaskDoneOpen : setTopicDoneOpen;

  // Optimistic moves: shown until the backend call (and the reload) settles.
  // Each move has its own operation id and only its own completion clears it;
  // moves of one card are sent one after another (lib/board.ts).
  const [taskMoves, setTaskMoves] = useState<PendingMoves<TaskStatus>>(new Map());
  const [taskLocations, setTaskLocations] = useState<PendingMoves<TaskLocation>>(new Map());
  const [topicMoves, setTopicMoves] = useState<PendingMoves<TopicStatus>>(new Map());
  const opSeq = useRef(0);
  const queue = useRef(createSerialQueue()).current;
  const [drag, setDrag] = useState<Drag | null>(null);
  const [menu, setMenu] = useState<{ anchor: HTMLElement; items: MenuItem[] } | null>(null);
  const viewportAnchor = useRef<ViewAnchor | null>(null);
  const dragCleanup = useRef<(() => void) | null>(null);
  const [topicChange, setTopicChange] = useState<TopicChange | null>(null);
  const [boardFloor, setBoardFloor] = useState<number | undefined>();
  const [landed, setLanded] = useState<{ id: string; op: number } | null>(null);
  const suppressClick = useRef(0);
  const boardRef = useRef<HTMLDivElement>(null);
  const [boardOverflow, setBoardOverflow] = useState(false);
  useLayoutEffect(() => {
    const el = boardRef.current;
    if (!el) return;
    const measure = () => setBoardOverflow(el.scrollWidth > el.clientWidth + 1);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [mode, doneOpen, topicArchiveOpen, scope, snap]);

  const taskCols = useMemo(() => groupTasks(snap, scope, moveOverrides(taskMoves), moveOverrides(taskLocations)), [snap, scope, taskMoves, taskLocations]);
  const topicCols = useMemo(() => groupTopics(snap, scope, view.today, moveOverrides(topicMoves)), [snap, scope, view.today, topicMoves, getLanguage()]);
  const counts = useMemo(() => countTasks(snap), [snap, getLanguage()]);
  const lanes = useMemo(() => taskLanes(taskCols, snap.topics, taskDoneOpen), [taskCols, snap.topics, taskDoneOpen]);

  const setMode = (m: BoardMode) => {
    setModeState(m);
    writePref(MODE_PREF, m);
    if (boardRef.current) boardRef.current.scrollLeft = 0;
  };
  const setScope = (s: string) => {
    setScopePref(s);
    writePref(SCOPE_PREF, s);
    if (boardRef.current) boardRef.current.scrollLeft = 0;
  };

  const cardElement = (id: string) => boardRef.current?.querySelector<HTMLElement>(`[data-card-id="${CSS.escape(id)}"]`);
  const cardSpot = (id: string, wantedY?: number): CardSpot => {
    const board = boardRef.current;
    const rect = board?.getBoundingClientRect();
    const card = cardElement(id)?.getBoundingClientRect();
    const top = (rect?.top ?? 0) + 44;
    const bottom = Math.max(top, (rect?.bottom ?? window.innerHeight) - Math.min(card?.height ?? 60, 180) - 8);
    const clamp = (y: number) => Math.max(top, Math.min(bottom, y));
    return { y: clamp(wantedY ?? card?.top ?? top), sourceY: clamp(card?.top ?? top), left: board?.scrollLeft ?? 0 };
  };

  // Run before painting, on both the optimistic layout and the backend refresh.
  // Browser focus and native scroll anchoring must not compete with this anchor.
  useLayoutEffect(() => {
    const anchor = viewportAnchor.current;
    const board = boardRef.current;
    if (!anchor || !board) return;
    const card = cardElement(anchor.id);
    if (card) {
      const scroller = card.closest<HTMLElement>(".col-body") ?? board;
      scroller.scrollTop = anchoredScrollTop(scroller.scrollTop, card.getBoundingClientRect().top, anchor.y, Math.max(0, scroller.scrollHeight - scroller.clientHeight));
      card.focus({ preventScroll: true });
      board.scrollLeft = anchor.left;
    }
    if (anchor.settled) requestAnimationFrame(() => { if (viewportAnchor.current === anchor) viewportAnchor.current = null; });
  });
  useEffect(() => {
    if (!landed) return;
    const timeout = window.setTimeout(() => setLanded(null), 1800);
    return () => window.clearTimeout(timeout);
  }, [landed]);
  useEffect(() => {
    setBoardFloor(undefined);
    viewportAnchor.current = null;
    return () => dragCleanup.current?.();
  }, [mode, scope]);

  const moveTask = async (task: Task, drop: TaskDrop, spot = cardSpot(drop.id)) => {
    const op = ++opSeq.current;
    viewportAnchor.current = { ...spot, id: drop.id, op, settled: false };
    setBoardFloor(boardRef.current?.querySelector<HTMLElement>(".board-lanes-content")?.offsetHeight);
    if (drop.status === "done") setTaskDoneOpen(true);
    setTaskMoves((m) => beginMove(m, drop.id, op, drop.status));
    if (drop.changesLocation) setTaskLocations((m) => beginMove(m, drop.id, op, drop.to));
    const result = await queue(`task:${drop.id}`, () => run((a) => applyTaskDrop(a, drop)));
    const anchor = viewportAnchor.current;
    if (anchor?.op === op) {
      anchor.settled = true;
      if (!result) anchor.y = anchor.sourceY;
    }
    setTaskMoves((m) => settleMove(m, drop.id, op));
    setTaskLocations((m) => settleMove(m, drop.id, op));
    if (result) {
      setLanded({ id: drop.id, op });
      if (drop.changesLocation) {
        const context = taskContext(snap, drop.to);
        notify(tr("Przeniesiono „{0}” do: {1}{2}.", task.title, context.project?.name ?? tr("Skrzynka"), context.topic ? ` / ${context.topic.title}` : ""));
      }
    }
  };

  const moveTopic = async (topic: Topic, to: TopicStatus, spot = cardSpot(topicId(topic))) => {
    if (topic.status === to) return;
    const id = topicId(topic);
    const op = ++opSeq.current;
    viewportAnchor.current = { ...spot, id, op, settled: false };
    if (to === "done") setTopicDoneOpen(true);
    setTopicMoves((m) => beginMove(m, id, op, to));
    const result = await queue(`topic:${id}`, () => run((a) => a.updateTopic(topic.project, topic.slug, { status: to })));
    if (viewportAnchor.current?.op === op) {
      viewportAnchor.current.settled = true;
      if (!result) viewportAnchor.current.y = viewportAnchor.current.sourceY;
    }
    setTopicMoves((m) => settleMove(m, id, op));
    if (result) setLanded({ id, op });
  };

  const findCard = (id: string): Task | Topic | undefined => mode === "tasks"
    ? TASK_COLUMNS.flatMap((c) => taskCols[c]).find((t) => t.id === id)
    : TOPIC_BOARD_COLUMNS.flatMap((c) => topicCols[c]).find((t) => topicId(t) === id);

  const moveCard = (id: string, to: string, target: TaskLocation | null = null, wantedY?: number) => {
    const card = findCard(id);
    if (!card) return;
    const spot = cardSpot(id, wantedY);
    if (mode === "tasks") {
      const task = card as Task;
      const drop = taskDrop(task, to as TaskStatus, target);
      if (!drop) return;
      if (drop.changesLocation) setTopicChange({ task, drop, spot });
      else void moveTask(task, drop, spot);
    } else {
      const topic = card as Topic;
      const from = topicCols.archive.some((t) => topicId(t) === id) ? "archive" : topic.status;
      const status = topicMoveColumns(from).find((s) => s === to);
      if (status) void moveTopic(topic, status, spot);
    }
  };

  const open = (card: Task | Topic) => {
    viewportAnchor.current = null;
    openDetail?.(mode === "tasks" ? taskTarget(card as Task) : topicTarget(card as Topic));
  };

  // ----- pointer drag -----
  const startDrag = (e: PointerEvent<HTMLElement>, id: string, from: string) => {
    if (topicChange || e.button !== 0 || (e.target as HTMLElement).closest("button:not(.card-open), a, input, select, textarea")) return;
    // Preserve a title click, but avoid text selection/native dragging mid-gesture.
    e.preventDefault();
    e.currentTarget.focus({ preventScroll: true });
    viewportAnchor.current = null;
    const el = e.currentTarget;
    const rect = el.getBoundingClientRect();
    const grabX = e.clientX - rect.left;
    const grabY = e.clientY - rect.top;
    const startX = e.clientX;
    const startY = e.clientY;
    let ghost: HTMLElement | null = null;
    let over: DropTarget | null = null;
    let active = false;
    let lastX = startX;
    let lastY = startY;
    let timer = 0;

    const hitTarget = () => {
      const hit = document.elementFromPoint(lastX, lastY) as HTMLElement | null;
      const board = boardRef.current;
      let next: DropTarget | null = null;
      if (hit && board?.contains(hit)) {
        const cell = hit.closest<HTMLElement>("[data-col]");
        let col = cell?.dataset.col;
        if (!col && mode === "tasks" && hit.closest("[data-lane]")) {
          col = Array.from(board.querySelectorAll<HTMLElement>(".board-lane-columns [data-col]")).find((head) => {
            const r = head.getBoundingClientRect();
            return lastX >= r.left && lastX <= r.right;
          })?.dataset.col;
        }
        if (col && (mode === "tasks" || (col !== from && topicMoveColumns(from as TopicColumn).some((s) => s === col)))) {
          const card = hit.closest<HTMLElement>("[data-card-id]");
          const place = card?.dataset.dropKind ? card : cell?.dataset.dropKind ? cell : hit.closest<HTMLElement>("[data-lane]");
          const project = place?.dataset.dropProject;
          const topic = place?.dataset.dropTopic;
          const location: TaskLocation | null = mode === "tasks" && project
            ? place?.dataset.dropKind === "topic" && topic ? { kind: "topic", project, topic }
              : place?.dataset.dropKind === "project" ? { kind: "project", project } : null
            : null;
          next = { col, location, lane: hit.closest<HTMLElement>("[data-lane]")?.dataset.lane ?? null, card: card?.dataset.cardId ?? null };
        }
      }
      if (JSON.stringify(next) !== JSON.stringify(over)) {
        over = next;
        setDrag((d) => d ? { ...d, over: next } : d);
      }
    };
    const autoScroll = () => {
      const b = boardRef.current;
      if (!b) return;
      const r = b.getBoundingClientRect();
      if (lastX < r.left + 56) b.scrollLeft -= 14;
      else if (lastX > r.right - 56) b.scrollLeft += 14;
      if (lastY < r.top + 56) b.scrollTop -= 14;
      else if (lastY > r.bottom - 56) b.scrollTop += 14;
      hitTarget();
    };
    const finish = (drop: boolean) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key, true);
      window.clearInterval(timer);
      dragCleanup.current = null;
      ghost?.remove();
      document.body.classList.remove("dragging-card");
      if (!active) return;
      suppressClick.current = performance.now() + 250;
      setDrag(null);
      if (drop && over) moveCard(id, over.col, over.location, lastY - grabY);
    };
    function move(ev: globalThis.PointerEvent) {
      lastX = ev.clientX;
      lastY = ev.clientY;
      if (!active) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 6) return;
        active = true;
        ghost = el.cloneNode(true) as HTMLElement;
        ghost.classList.add("board-ghost");
        ghost.removeAttribute("data-card-id");
        ghost.style.width = `${rect.width}px`;
        document.body.appendChild(ghost);
        document.body.classList.add("dragging-card");
        timer = window.setInterval(autoScroll, 24);
        setDrag({ id, from, over: null });
      }
      ghost!.style.transform = `translate(${ev.clientX - grabX}px, ${ev.clientY - grabY}px)`;
      hitTarget();
    }
    function up() {
      finish(true);
    }
    function cancel() {
      finish(false);
    }
    function key(ev: globalThis.KeyboardEvent) {
      if (ev.key === "Escape" && active) {
        ev.stopPropagation();
        finish(false);
      }
    }
    dragCleanup.current = () => finish(false);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key, true);
  };

  // ----- keyboard -----
  const columns: readonly Status[] = mode === "tasks" ? TASK_COLUMNS : TOPIC_COLUMNS;
  const statusKeys: Record<string, Status> =
    mode === "tasks"
      ? Object.fromEntries(TASK_COLUMNS.map((s) => [TASK_STATUS_KEY[s], s]))
      : Object.fromEntries(TOPIC_STATUSES.map((s, i) => [String(i + 1), s]));

  const cardKeys = (e: KeyboardEvent<HTMLElement>, id: string, col: string, card: Task | Topic) => {
    const titleFocused = (e.target as HTMLElement).closest(".card-open") !== null;
    if ((e.target !== e.currentTarget && !titleFocused) || e.metaKey || e.ctrlKey) return;
    const k = e.key;
    const moveColumns = mode === "tasks" ? TASK_COLUMNS : topicMoveColumns(col as TopicColumn);
    if (titleFocused && (k === "Enter" || k === " ")) return;
    if (e.altKey && (k === "ArrowLeft" || k === "ArrowRight")) {
      const next = adjacentColumn<string>(col === "archive" ? [...moveColumns, "archive"] : moveColumns, col, k === "ArrowRight" ? 1 : -1);
      if (next) moveCard(id, next);
    } else if (e.altKey) {
      return;
    } else if (k === "Enter" || k === " ") open(card);
    else if (statusKeys[k]) moveCard(id, statusKeys[k]!);
    else if (k === "m") {
      const label = mode === "tasks" ? TASK_STATUS_LABEL : TOPIC_STATUS_LABEL;
      const keyOf = Object.fromEntries(Object.entries(statusKeys).map(([key, s]) => [s, key]));
      setMenu({
        anchor: e.currentTarget,
        items: moveColumns.map((s) => ({
          key: s,
          label: (label as Record<string, string>)[s]!,
          hint: keyOf[s],
          checked: s === col,
          onSelect: () => moveCard(id, s),
        })),
      });
    } else if (k === "ArrowDown" || k === "j") focusNeighbor(e.currentTarget, "down");
    else if (k === "ArrowUp" || k === "k") focusNeighbor(e.currentTarget, "up");
    else if (k === "ArrowRight" || k === "l") focusNeighbor(e.currentTarget, "right");
    else if (k === "ArrowLeft" || k === "h") focusNeighbor(e.currentTarget, "left");
    else return;
    e.preventDefault();
  };

  // ----- render -----
  const projects = partitionProjects(snap.projects).visible;
  const total = mode === "tasks" ? TASK_COLUMNS.reduce((n, c) => n + taskCols[c].length, 0) : TOPIC_BOARD_COLUMNS.reduce((n, c) => n + topicCols[c].length, 0);
  const scopeProject = scope === ALL_PROJECTS ? null : snap.projects.find((p) => p.slug === scope);

  const renderTask = (task: Task, grouped = false) => {
    const id = task.id ?? "";
    return <TaskCard
      key={id || `${task.location.kind}:${task.line}`}
      task={task}
      grouped={grouped}
      showProject={scope === ALL_PROJECTS}
      id={id}
      col={task.status}
      dragging={drag?.id === id}
      landed={landed?.id === id}
      dropTarget={drag?.over?.card === id && drag.id !== id}
      onMenu={(anchor) => setMenu({ anchor, items: columns.map((status) => ({ key: status, label: mode === "tasks" ? TASK_STATUS_LABEL[status as TaskStatus] : TOPIC_STATUS_LABEL[status as TopicStatus], checked: status === task.status, onSelect: () => moveCard(id, status) })) })}
      onOpen={() => performance.now() > suppressClick.current && open(task)}
      onPointerDown={(e) => { if (id) startDrag(e, id, task.status); }}
      onKeyDown={(e) => { if (id) cardKeys(e, id, task.status, task); }}
    />;
  };

  const renderColumn = (col: TopicColumn): ReactNode => {
    const items = topicCols[col];
    const archive = col === "archive";
    const collapsible = archive || col === "done";
    const collapsed = archive ? !topicArchiveOpen : col === "done" && !topicDoneOpen;
    const { shown, hidden } = capColumn(items, collapsible);
    const label = archive ? tr("Archiwum") : col === "done" ? tr("Zrobione") : TOPIC_STATUS_LABEL[col];
    const canDrop = !!drag && drag.from !== col && topicMoveColumns(drag.from as TopicColumn).some((s) => s === col);
    const head = <>{col === "done" || archive ? label : <TopicStatusBadge status={col} />}<span className="count">{items.length}</span></>;
    return <section key={col} className={`col${collapsed ? " collapsed" : ""}${canDrop && drag?.over?.col === col ? " over" : ""}${canDrop ? " droppable" : ""}${drag && (archive || (drag.from === "archive" && col === "done")) ? " no-drop" : ""}`} data-col={col} aria-label={label}>
      {collapsible ? <button type="button" className="col-head col-toggle" data-col-head aria-expanded={!collapsed} onClick={() => archive ? setTopicArchiveOpen(!topicArchiveOpen) : setTopicDoneOpen(!topicDoneOpen)} title={archive ? tr("Tematy zrobione co najmniej {0} dni temu według ostatniej aktualizacji. Archiwizacja jest automatyczna.", TOPIC_ARCHIVE_DAYS) : undefined}><ChevronRight size={12} className="chev" aria-hidden />{head}</button> : <h2 className="col-head" data-col-head tabIndex={-1}>{head}</h2>}
      {drag && archive && <p className="col-empty archive-hint">{tr("Archiwizacja automatyczna po")} {TOPIC_ARCHIVE_DAYS}  {tr("dniach · nie można tu upuścić karty")}</p>}
      {!collapsed && <div className="col-body">
        {!shown.length && <p className="col-empty">{tr("Brak tematów")}</p>}
        {shown.map((topic) => {
          const id = topicId(topic);
          const count = counts.topic.get(topicKey(topic.project, topic.slug));
          return <TopicCard key={id} topic={topic} open={count?.open ?? 0} total={count?.total ?? 0} showProject={scope === ALL_PROJECTS} id={id} col={col} dragging={drag?.id === id} landed={landed?.id === id} dropTarget={canDrop && drag?.over?.card === id && drag.id !== id} onMenu={(anchor) => setMenu({ anchor, items: topicMoveColumns(col).map((status) => ({ key: status, label: TOPIC_STATUS_LABEL[status], checked: status === col, onSelect: () => moveCard(id, status) })) })} onOpen={() => performance.now() > suppressClick.current && open(topic)} onPointerDown={(e) => startDrag(e, id, col)} onKeyDown={(e) => cardKeys(e, id, col, topic)} />;
        })}
        {hidden > 0 && <p className="col-more">{tr("Pokazano")} {DONE_CAP}  {tr("najnowszych z")} {items.length}.</p>}
      </div>}
    </section>;
  };

  return (
    <div
      className="board-view"
      data-mode={mode}
      style={scopeProject?.color ? ({ "--accent": scopeProject.color } as CSSProperties) : undefined}
    >
      <header className="board-head">
        <h1 className="view-title">{tr("Tablica")}</h1>
        <div className="board-controls">
          <Segmented<BoardMode>
            label={tr("Tryb tablicy")}
            value={mode}
            onChange={setMode}
            options={[
              { value: "tasks", label: tr("Zadania") },
              { value: "topics", label: tr("Tematy") },
            ]}
          />
          <select
            className="field select board-scope"
            style={scopeProject?.color ? { color: scopeProject.color } : undefined}
            aria-label={tr("Zakres tablicy")}
            value={scope}
            onChange={(e) => setScope(e.target.value)}
          >
            <option value={ALL_PROJECTS}>{tr("Wszystkie aktywne projekty")}</option>
            {projects.map((p) => (
              <option key={p.slug} value={p.slug} style={p.color ? { color: p.color } : undefined}>
                {p.name}
                {p.state === "paused" ? tr(" (wstrzymany)") : ""}
              </option>
            ))}
          </select>
        </div>
      </header>

      <div className="board-help" id="board-help">
        <span>{tr("Przeciągnij kartę lub jej tytuł · ⌥←/⌥→ — przenieś · Enter — szczegóły · m — zmień status")}</span>
        {boardOverflow && <span className="board-scroll-controls">{tr("Więcej kolumn")}<button type="button" className="btn btn-quiet" aria-label={tr("Przewiń kolumny w lewo")} onClick={() => boardRef.current?.scrollBy({ left: -260, behavior: "auto" })}>←</button>
          <button type="button" className="btn btn-quiet" aria-label={tr("Przewiń kolumny w prawo")} onClick={() => boardRef.current?.scrollBy({ left: 260, behavior: "auto" })}>→</button>
        </span>}
      </div>
      {total === 0 ? (
        <div className="empty board-empty">
          <p className="empty-title">{mode === "tasks" ? tr("Brak zadań w tym zakresie.") : tr("Brak tematów w tym zakresie.")}</p>
          <p className="empty-text">
            {mode === "tasks"
              ? tr("Zadania ze skrzynki nie trafiają na tablicę — przenieś je do projektu lub tematu.")
              : tr("Temat to jeden wątek pracy w projekcie. Dodasz go w widoku projektu.")}
          </p>
        </div>
      ) : mode === "tasks" ? (
        <div className="board board-lanes" onWheel={() => { viewportAnchor.current = null; }} ref={boardRef} role="group" aria-label={tr("Zadania według tematów i statusów")} style={{ "--board-min-width": doneOpen ? "960px" : "860px", "--board-columns": `repeat(3, minmax(230px, 1fr)) ${doneOpen ? "minmax(230px, 1fr)" : "130px"}` } as CSSProperties}>
          <div className="board-lanes-content" style={{ minHeight: boardFloor }}>
            <div className="board-column-tracks" aria-hidden>{TASK_COLUMNS.map((col) => <div key={col} className="board-column-track" />)}</div>
            <div className="board-lane-columns">
              {TASK_COLUMNS.map((col) => <div key={col} data-col={col}>
                {col === "done" ? <button type="button" className="col-head col-toggle" data-col-head aria-expanded={doneOpen} onClick={() => setDoneOpen(!doneOpen)}><ChevronRight size={12} className="chev" aria-hidden />{TASK_STATUS_LABEL[col]} <span className="count">{taskCols[col].length}</span></button> : <h2 className="col-head" data-col-head tabIndex={-1}><span className={`status status-${col} col-glyph`}><StatusGlyph status={col} /></span>{TASK_STATUS_LABEL[col]} <span className="count">{taskCols[col].length}</span></h2>}
              </div>)}
            </div>
            {lanes.map((lane) => <section key={lane.key} data-lane={lane.key} data-drop-kind={lane.topic ? "topic" : undefined} data-drop-project={lane.topic?.project} data-drop-topic={lane.topic?.slug} className={`board-lane${lane.topic ? " grouped" : " ungrouped"}`} aria-label={lane.topic ? tr("Temat: {0}", lane.topic.title) : tr("Pojedyncze zadania")}>
              <header className="board-lane-head">
                {lane.topic ? <>
                  <span className="board-lane-kind">{tr("Temat")}</span>
                  <button type="button" className="board-lane-title" onClick={() => openDetail?.(topicTarget(lane.topic!))}>{lane.topic.title}</button>
                  <TopicStatusPill project={{ slug: lane.topic.project }} topic={lane.topic} />
                  {scope === ALL_PROJECTS && <span className="board-lane-project"><ProjectName name={snap.projects.find((p) => p.slug === lane.topic!.project)?.name ?? lane.topic.project} color={snap.projects.find((p) => p.slug === lane.topic!.project)?.color} /></span>}
                </> : <h2 className="board-lane-single-title">{tr("Pojedyncze zadania")}</h2>}
                <span className="board-lane-count">{lane.total} {plural(lane.total, tr("zadanie"), tr("zadania"), tr("zadań"))}</span>
              </header>
              <div className="board-lane-cells">
                {TASK_COLUMNS.map((col) => <div key={col} data-col={col} data-drop-kind={lane.topic ? "topic" : undefined} data-drop-project={lane.topic?.project} data-drop-topic={lane.topic?.slug} className={`board-lane-cell${drag ? " droppable" : ""}${drag?.over?.col === col && drag.over.lane === lane.key ? " over" : ""}`} aria-label={TASK_STATUS_LABEL[col]}>
                  {lane.columns[col].map((task) => renderTask(task, !!lane.topic))}
                  {col === "done" && lane.hiddenDone > 0 && (doneOpen ? <span className="board-hidden-done">+{lane.hiddenDone}  {tr("poza limitem podglądu")}</span> : <button type="button" className="board-hidden-done" onClick={() => setDoneOpen(true)}>{lane.hiddenDone} {plural(lane.hiddenDone, tr("zrobione"), tr("zrobione"), tr("zrobionych"))}  {tr("· pokaż")}</button>)}
                </div>)}
              </div>
            </section>)}
            {lanes.length === 0 && <p className="col-empty">{tr("Wszystkie zadania są zrobione. Rozwiń kolumnę „Zrobione”, żeby je zobaczyć.")}</p>}
            {doneOpen && taskCols.done.length > DONE_CAP && <p className="col-more">{tr("Pokazano")} {DONE_CAP}  {tr("najnowszych zrobionych zadań z")} {taskCols.done.length}.</p>}
          </div>
        </div>
      ) : (
        <div className="board" ref={boardRef} role="group" aria-label={tr("Tematy według statusu")}>
          {TOPIC_BOARD_COLUMNS.map(renderColumn)}
        </div>
      )}
      {drag?.over && <div className="board-drop-hint" role="status">{tr("Upuść:")} {mode === "tasks" ? TASK_STATUS_LABEL[drag.over.col as TaskStatus] : TOPIC_STATUS_LABEL[drag.over.col as TopicStatus]}{drag.over.location?.kind === "topic" && tr(" · temat: {0}", taskContext(snap, drag.over.location).topic?.title ?? drag.over.location.topic)}</div>}
      {topicChange && <TopicChangeConfirm change={topicChange} snapshot={snap} onCancel={() => {
        setTopicChange(null);
        cardElement(topicChange.drop.id)?.focus({ preventScroll: true });
      }} onConfirm={() => {
        const change = topicChange;
        setTopicChange(null);
        void moveTask(change.task, change.drop, change.spot);
      }} />}
      {menu && (
        <Menu
          anchor={menu.anchor}
          label={tr("Przenieś do kolumny")}
          items={menu.items}
          onClose={() => setMenu(null)}
          minWidth={190}
        />
      )}
    </div>
  );
}

function TopicChangeConfirm({ change, snapshot, onCancel, onConfirm }: {
  change: TopicChange;
  snapshot: Snapshot;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialog = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => { cancel.current?.focus({ preventScroll: true }); }, []);
  const place = (location: TaskLocation) => {
    const context = taskContext(snapshot, location);
    return <><ProjectName name={context.project?.name ?? tr("Skrzynka")} color={context.project?.color} /><span>{context.topic?.title ?? (location.kind === "topic" ? location.topic : tr("Zadania ogólne · bez tematu"))}</span></>;
  };
  return createPortal(<div className="board-confirm-backdrop" onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}>
    <div className="board-confirm" ref={dialog} role="alertdialog" aria-modal="true" aria-labelledby="board-confirm-title" aria-describedby="board-confirm-description" onKeyDown={(e) => {
      e.stopPropagation();
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onCancel(); }
      if (e.key === "Tab") {
        const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>("button");
        if (!buttons?.length) return;
        const first = buttons[0]!;
        const last = buttons[buttons.length - 1]!;
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus({ preventScroll: true }); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus({ preventScroll: true }); }
      }
    }}>
      <h2 id="board-confirm-title">{tr("Zmienić temat zadania?")}</h2>
      <p id="board-confirm-description">{tr("Przenosisz zadanie „")}{change.task.title}”.</p>
      <dl className="board-confirm-places">
        <div><dt>{tr("Z")}</dt><dd>{place(change.drop.from)}</dd></div>
        <div><dt>{tr("Do")}</dt><dd>{place(change.drop.to)}</dd></div>
      </dl>
      {change.task.status !== change.drop.status && <p>{tr("Status po przeniesieniu:")} <strong>{TASK_STATUS_LABEL[change.drop.status]}</strong>.</p>}
      <div className="board-confirm-actions"><Button ref={cancel} onClick={onCancel}>{tr("Anuluj")}</Button><Button variant="primary" onClick={onConfirm}>{tr("Przenieś zadanie")}</Button></div>
    </div>
  </div>, document.body);
}

function focusNeighbor(from: HTMLElement, dir: "up" | "down" | "left" | "right") {
  const board = from.closest<HTMLElement>(".board");
  if (!board) return;
  const status = from.dataset.cardCol;
  const all = Array.from(board.querySelectorAll<HTMLElement>("[data-card-id]"));
  const cards = all.filter((card) => card.dataset.cardCol === status);
  const i = cards.indexOf(from);
  if (dir === "up" || dir === "down") {
    cards[i + (dir === "down" ? 1 : -1)]?.focus();
    return;
  }
  const cols = [...new Set(Array.from(board.querySelectorAll<HTMLElement>("[data-col]")).map((el) => el.dataset.col))];
  let c = cols.indexOf(status) + (dir === "right" ? 1 : -1);
  for (; c >= 0 && c < cols.length; c += dir === "right" ? 1 : -1) {
    const there = all.filter((card) => card.dataset.cardCol === cols[c]);
    if (there.length) {
      const lane = from.closest("[data-lane]");
      (there.find((card) => lane && card.closest("[data-lane]") === lane) ?? there[Math.min(i, there.length - 1)])?.focus();
      return;
    }
  }
}

interface ShellProps {
  id: string;
  col: string;
  dragging: boolean;
  landed?: boolean;
  dropTarget?: boolean;
  location?: TaskLocation;
  onOpen: () => void;
  onMenu: (anchor: HTMLElement) => void;
  onPointerDown: (e: PointerEvent<HTMLElement>) => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
}

function Card({
  id,
  col,
  dragging,
  landed,
  dropTarget,
  location,
  onOpen,
  onMenu,
  onPointerDown,
  onKeyDown,
  label,
  header,
  children,
}: ShellProps & { label: string; header: ReactNode; children?: ReactNode }) {
  return (
    <article
      className={`card${dragging ? " dragging" : ""}${landed ? " landed" : ""}${dropTarget ? " drop-target" : ""}`}
      tabIndex={0}
      data-card-id={id}
      data-card-col={col}
      data-drop-kind={location?.kind}
      data-drop-project={location?.kind !== "inbox" ? location?.project : undefined}
      data-drop-topic={location?.kind === "topic" ? location.topic : undefined}
      aria-label={label}
      aria-roledescription="karta"
      onClick={onOpen}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
    >
      <div className="card-head">
        {header}
        <button type="button" className="icon-btn card-menu" aria-label={tr("Zmień status: {0}", label)} title={tr("Zmień status (m)")} onClick={(e) => { e.stopPropagation(); onMenu(e.currentTarget); }}>
          <MoreHorizontal size={14} />
        </button>
      </div>
      {children}
    </article>
  );
}

function TaskCard({ task, showProject, grouped = false, ...shell }: ShellProps & { task: Task; showProject: boolean; grouped?: boolean }) {
  const view = useView();
  const ctx = taskContext(view.snapshot, task.location);
  const openDetail = useOpenDetail();
  const overdue = task.status !== "done" && isOverdue(task.due, view.today);
  return (
    <Card {...shell} location={task.location} label={`${task.title}. ${TASK_STATUS_LABEL[task.status]}${task.due ? tr(". Termin {0}", formatDue(task.due, view.today)) : ""}`} header={<button type="button" className="card-title card-open" title={task.title} onClick={(e) => { e.stopPropagation(); shell.onOpen(); }}>{task.title}</button>}>
      {(task.due || task.links.length > 0 || task.priority !== "normal") && (
        <div className="card-meta">
          {task.priority === "high" && <span className="priority-label">{tr("Wysoki")}</span>}
          {task.due && (
            <span className={`due${overdue ? " overdue" : task.status !== "done" && task.due.slice(0, 10) === view.today ? " due-today" : ""}`} title={tr("Termin: {0}{1}", task.due.replace("T", " "), overdue ? tr(" — po terminie") : "")}>
              {overdue ? tr("Po terminie · ") : ""}{formatDue(task.due, view.today)}
            </span>
          )}
          {task.priority === "low" && <ArrowDown size={12} className="meta-icon" aria-label={tr("Niski priorytet")} />}
          {task.links[0] && <LinkChip url={task.links[0]} />}
          {task.links.length > 1 && <span className="meta-more">+{task.links.length - 1}</span>}
        </div>
      )}
      {!grouped && <div className="card-ctx" title={`${ctx.project?.name ?? ""}${ctx.topic ? ` · ${ctx.topic.title}` : ""}`}>
        {(showProject || !ctx.topic) && ctx.project && <ProjectName name={ctx.project.name} color={ctx.project.color} />}
      </div>}
      {!grouped && (ctx.topic ? <button type="button" className="card-topic" onClick={(e) => { e.stopPropagation(); openDetail?.(topicTarget(ctx.topic!)); }} title={tr("Otwórz temat i zobacz jego zadania")}>
        <span>{tr("Temat:")} {ctx.topic.title}</span>
      </button> : <span className="card-no-topic">{tr("Zadanie ogólne · bez tematu")}</span>)}
    </Card>
  );
}

function TopicCard({
  topic,
  open,
  total,
  showProject,
  ...shell
}: ShellProps & { topic: Topic; open: number; total: number; showProject: boolean }) {
  const view = useView();
  const project = view.snapshot.projects.find((p) => p.slug === topic.project);
  const line = topicSummary(topic.summary).lead;
  return (
    <Card
      {...shell}
      label={`${topic.title}. ${TOPIC_STATUS_LABEL[topic.status]}. ${openCount(open, total)}`}
      header={
        <button type="button" className="card-title card-open" title={topic.title} onClick={(e) => { e.stopPropagation(); shell.onOpen(); }}>
          {topic.title}
        </button>
      }
    >
      {line && <p className="card-summary">{line}</p>}
      <div className="card-foot">
        <span className="card-ctx" title={project?.name}>
          {showProject && <ProjectName name={project?.name ?? topic.project} color={project?.color} />}
        </span>
        <span className="count" title={openCount(open, total)} aria-label={openCount(open, total)}>
          {total === 0 ? tr("Brak zadań") : tr("{0} otw. · {1} zrob.", open, Math.max(0, total - open))}
        </span>
        {topic.updated && (
          <span className="age" title={tr("Zaktualizowano {0}", topic.updated.replace("T", " "))}>
            {formatAge(topic.updated, view.today)}
          </span>
        )}
      </div>
    </Card>
  );
}

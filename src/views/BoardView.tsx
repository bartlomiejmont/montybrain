// „Tablica": kanban over task statuses or topic statuses. Cards are dragged with
// pointer events (no dependency) and can be moved with the keyboard
// (⌥←/⌥→, 1…5 or `m`). Moves are shown immediately and reconciled with the
// reloaded state; a failed move rolls back through the usual error toast.

import { ChevronRight, ArrowDown } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";

import type { Task, TaskStatus, Topic, TopicStatus } from "../api";
import { LinkChip } from "../components/TaskRow";
import { Menu, type MenuItem } from "../components/Menu";
import { StatusGlyph } from "../components/StatusToggle";
import { TopicStatusBadge } from "../components/TopicItem";
import { Segmented, ProjectDot, useStoredFlag } from "../components/ui";
import {
  ALL_PROJECTS,
  DONE_CAP,
  TASK_COLUMNS,
  TOPIC_COLUMNS,
  adjacentColumn,
  beginMove,
  capColumn,
  createSerialQueue,
  effectiveScope,
  firstLine,
  groupTasks,
  groupTopics,
  moveOverrides,
  settleMove,
  topicId,
  type BoardMode,
  type PendingMoves,
} from "../lib/board";
import { formatAge, formatDue, isOverdue } from "../lib/dates";
import { countTasks, partitionProjects, taskContext, topicKey } from "../lib/group";
import { TASK_STATUS_KEY, TASK_STATUS_LABEL, TOPIC_STATUSES, TOPIC_STATUS_LABEL } from "../lib/labels";
import { taskHref, topicHref } from "../lib/nav";
import { navigate } from "../router/router";
import { readPref, revealTask, revealTopic, writePref } from "../store/ui";
import { useVault, useView } from "../store/vault";

type Status = TaskStatus | TopicStatus;

interface Drag {
  id: string;
  from: string;
  over: string | null;
}

const MODE_PREF = "morwisko.board.mode";
const SCOPE_PREF = "morwisko.board.scope";

export function BoardView() {
  const view = useView();
  const { notify, run } = useVault();
  const snap = view.snapshot;

  const [mode, setModeState] = useState<BoardMode>(() => (readPref(MODE_PREF) === "topics" ? "topics" : "tasks"));
  const [scopePref, setScopePref] = useState<string>(() => readPref(SCOPE_PREF) ?? ALL_PROJECTS);
  const scope = effectiveScope(snap, scopePref);
  const [taskDoneOpen, setTaskDoneOpen] = useStoredFlag("morwisko.board.done.tasks", false);
  const [topicDoneOpen, setTopicDoneOpen] = useStoredFlag("morwisko.board.done.topics", false);
  const doneOpen = mode === "tasks" ? taskDoneOpen : topicDoneOpen;
  const setDoneOpen = mode === "tasks" ? setTaskDoneOpen : setTopicDoneOpen;

  // Optimistic moves: shown until the backend call (and the reload) settles.
  // Each move has its own operation id and only its own completion clears it;
  // moves of one card are sent one after another (lib/board.ts).
  const [taskMoves, setTaskMoves] = useState<PendingMoves<TaskStatus>>(new Map());
  const [topicMoves, setTopicMoves] = useState<PendingMoves<TopicStatus>>(new Map());
  const opSeq = useRef(0);
  const queue = useRef(createSerialQueue()).current;
  const [drag, setDrag] = useState<Drag | null>(null);
  const [menu, setMenu] = useState<{ anchor: HTMLElement; items: MenuItem[] } | null>(null);
  const focusAfter = useRef<{ id: string; col: string } | null>(null);
  const suppressClick = useRef(0);
  const boardRef = useRef<HTMLDivElement>(null);

  const taskCols = useMemo(() => groupTasks(snap, scope, moveOverrides(taskMoves)), [snap, scope, taskMoves]);
  const topicCols = useMemo(() => groupTopics(snap, scope, moveOverrides(topicMoves)), [snap, scope, topicMoves]);
  const counts = useMemo(() => countTasks(snap), [snap]);

  const setMode = (m: BoardMode) => {
    setModeState(m);
    writePref(MODE_PREF, m);
  };
  const setScope = (s: string) => {
    setScopePref(s);
    writePref(SCOPE_PREF, s);
  };

  // `task.status` / `topic.status` here is what the card shows (pending move included).
  const moveTask = async (task: Task, to: TaskStatus) => {
    if (task.status === to || !task.id) {
      if (!task.id) notify("To zadanie nie ma jeszcze identyfikatora — za chwilę zostanie nadany.");
      return;
    }
    const id = task.id;
    const op = ++opSeq.current;
    setTaskMoves((m) => beginMove(m, id, op, to));
    // reloads the state; shows a toast on failure
    await queue(`task:${id}`, () => run((a) => a.updateTask(id, { status: to })));
    setTaskMoves((m) => settleMove(m, id, op));
  };

  const moveTopic = async (topic: Topic, to: TopicStatus) => {
    if (topic.status === to) return;
    const id = topicId(topic);
    const op = ++opSeq.current;
    setTopicMoves((m) => beginMove(m, id, op, to));
    await queue(`topic:${id}`, () => run((a) => a.updateTopic(topic.project, topic.slug, { status: to })));
    setTopicMoves((m) => settleMove(m, id, op));
  };

  const findCard = (id: string): Task | Topic | undefined =>
    mode === "tasks"
      ? TASK_COLUMNS.flatMap((c) => taskCols[c]).find((t) => t.id === id)
      : TOPIC_COLUMNS.flatMap((c) => topicCols[c]).find((t) => topicId(t) === id);

  const moveCard = (id: string, to: string, keepFocus = false) => {
    const card = findCard(id);
    if (!card) return;
    if (keepFocus) focusAfter.current = { id, col: to };
    if (mode === "tasks") void moveTask(card as Task, to as TaskStatus);
    else void moveTopic(card as Topic, to as TopicStatus);
  };

  // Keyboard moves remount the card in another column: give focus back to it.
  useEffect(() => {
    const f = focusAfter.current;
    if (!f) return;
    const el = boardRef.current?.querySelector<HTMLElement>(`[data-card-id="${CSS.escape(f.id)}"]`);
    if (el) el.focus();
    else boardRef.current?.querySelector<HTMLElement>(`[data-col="${f.col}"] [data-col-head]`)?.focus();
    focusAfter.current = null;
  });

  const open = (card: Task | Topic) => {
    if (mode === "tasks") {
      const t = card as Task;
      const loc = t.location;
      revealTask(t.id, loc.kind === "inbox" ? null : loc.project, loc.kind === "topic" ? loc.topic : null);
      navigate(taskHref(t));
    } else {
      const t = card as Topic;
      revealTopic(t.project, t.slug);
      navigate(topicHref(t));
    }
  };

  // ----- pointer drag -----
  const startDrag = (e: PointerEvent<HTMLElement>, id: string, from: string) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, a, input, select, textarea")) return;
    const el = e.currentTarget;
    const rect = el.getBoundingClientRect();
    const grabX = e.clientX - rect.left;
    const grabY = e.clientY - rect.top;
    const startX = e.clientX;
    const startY = e.clientY;
    let ghost: HTMLElement | null = null;
    let over: string | null = null;
    let active = false;
    let lastX = startX;
    let timer = 0;

    const autoScroll = () => {
      const b = boardRef.current;
      if (!b) return;
      const r = b.getBoundingClientRect();
      if (lastX < r.left + 56) b.scrollLeft -= 14;
      else if (lastX > r.right - 56) b.scrollLeft += 14;
    };
    const finish = (drop: boolean) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key, true);
      window.clearInterval(timer);
      ghost?.remove();
      document.body.classList.remove("dragging-card");
      if (!active) return;
      suppressClick.current = performance.now() + 250;
      setDrag(null);
      if (drop && over && over !== from) moveCard(id, over, true);
    };
    function move(ev: globalThis.PointerEvent) {
      lastX = ev.clientX;
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
      const col = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-col]")?.dataset.col ?? null;
      if (col !== over) {
        over = col;
        setDrag((d) => (d ? { ...d, over: col } : d));
      }
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
    if (e.target !== e.currentTarget || e.metaKey || e.ctrlKey) return;
    const k = e.key;
    if (e.altKey && (k === "ArrowLeft" || k === "ArrowRight")) {
      const next = adjacentColumn(columns, col as Status, k === "ArrowRight" ? 1 : -1);
      if (next) moveCard(id, next, true);
    } else if (e.altKey) {
      return;
    } else if (k === "Enter" || k === " ") open(card);
    else if (statusKeys[k]) moveCard(id, statusKeys[k]!, true);
    else if (k === "m") {
      const label = mode === "tasks" ? TASK_STATUS_LABEL : TOPIC_STATUS_LABEL;
      const keyOf = Object.fromEntries(Object.entries(statusKeys).map(([key, s]) => [s, key]));
      setMenu({
        anchor: e.currentTarget,
        items: (columns as Status[]).map((s) => ({
          key: s,
          label: (label as Record<string, string>)[s]!,
          hint: keyOf[s],
          checked: s === col,
          onSelect: () => moveCard(id, s, true),
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
  const total = mode === "tasks" ? TASK_COLUMNS.reduce((n, c) => n + taskCols[c].length, 0) : TOPIC_COLUMNS.reduce((n, c) => n + topicCols[c].length, 0);
  const scopeProject = scope === ALL_PROJECTS ? null : snap.projects.find((p) => p.slug === scope);

  const renderColumn = (col: Status): ReactNode => {
    const items = (mode === "tasks" ? taskCols[col as TaskStatus] : topicCols[col as TopicStatus]) as (Task | Topic)[];
    const isDone = col === "done";
    const collapsed = isDone && !doneOpen;
    const { shown, hidden } = capColumn(items, isDone);
    const label = mode === "tasks" ? TASK_STATUS_LABEL[col as TaskStatus] : TOPIC_STATUS_LABEL[col as TopicStatus];
    const head = (
      <>
        {mode === "tasks" ? (
          <>
            <span className={`status status-${col} col-glyph`}>
              <StatusGlyph status={col as TaskStatus} />
            </span>
            <span className="col-label">{label}</span>
          </>
        ) : (
          <TopicStatusBadge status={col as TopicStatus} />
        )}
        <span className="count">{items.length}</span>
      </>
    );
    return (
      <section
        key={col}
        className={`col${collapsed ? " collapsed" : ""}${drag?.over === col ? " over" : ""}${drag && drag.from !== col ? " droppable" : ""}`}
        data-col={col}
        aria-label={label}
      >
        {isDone ? (
          <button
            type="button"
            className="col-head col-toggle"
            data-col-head
            aria-expanded={!collapsed}
            title={collapsed ? "Pokaż zrobione" : "Zwiń zrobione"}
            onClick={() => setDoneOpen(!doneOpen)}
          >
            <ChevronRight size={12} className="chev" />
            {head}
          </button>
        ) : (
          <h2 className="col-head" data-col-head tabIndex={-1}>
            {head}
          </h2>
        )}
        {!collapsed && (
          <div className="col-body">
            {shown.length === 0 && <p className="col-empty">{mode === "tasks" ? "Brak zadań" : "Brak tematów"}</p>}
            {shown.map((card) => {
              const id = mode === "tasks" ? ((card as Task).id ?? "") : topicId(card as Topic);
              const common = {
                id,
                col,
                dragging: drag?.id === id,
                onOpen: () => performance.now() > suppressClick.current && open(card),
                onPointerDown: (e: PointerEvent<HTMLElement>) => id && startDrag(e, id, col),
                onKeyDown: (e: KeyboardEvent<HTMLElement>) => id && cardKeys(e, id, col, card),
              };
              return mode === "tasks" ? (
                <TaskCard key={id || `l${(card as Task).line}`} task={card as Task} showProject={scope === ALL_PROJECTS} {...common} />
              ) : (
                <TopicCard
                  key={id}
                  topic={card as Topic}
                  open={counts.topic.get(topicKey((card as Topic).project, (card as Topic).slug))?.open ?? 0}
                  total={counts.topic.get(topicKey((card as Topic).project, (card as Topic).slug))?.total ?? 0}
                  showProject={scope === ALL_PROJECTS}
                  {...common}
                />
              );
            })}
            {hidden > 0 && (
              <p className="col-more">
                Pokazano {DONE_CAP} najnowszych z {items.length}.
              </p>
            )}
          </div>
        )}
      </section>
    );
  };

  return (
    <div
      className="board-view"
      style={scopeProject?.color ? ({ "--accent": scopeProject.color } as CSSProperties) : undefined}
    >
      <header className="board-head">
        <h1 className="view-title">Tablica</h1>
        <div className="board-controls">
          <Segmented<BoardMode>
            label="Tryb tablicy"
            value={mode}
            onChange={setMode}
            options={[
              { value: "tasks", label: "Zadania" },
              { value: "topics", label: "Tematy" },
            ]}
          />
          <select
            className="field select board-scope"
            aria-label="Zakres tablicy"
            value={scope}
            onChange={(e) => setScope(e.target.value)}
          >
            <option value={ALL_PROJECTS}>Wszystkie aktywne projekty</option>
            {projects.map((p) => (
              <option key={p.slug} value={p.slug}>
                {p.name}
                {p.state === "paused" ? " (wstrzymany)" : ""}
              </option>
            ))}
          </select>
        </div>
      </header>

      {total === 0 ? (
        <div className="empty board-empty">
          <p className="empty-title">{mode === "tasks" ? "Brak zadań w tym zakresie." : "Brak tematów w tym zakresie."}</p>
          <p className="empty-text">
            {mode === "tasks"
              ? "Zadania ze skrzynki nie trafiają na tablicę — przenieś je do projektu lub tematu."
              : "Temat to jeden wątek pracy w projekcie. Dodasz go w widoku projektu."}
          </p>
        </div>
      ) : (
        <div className="board" ref={boardRef} role="group" aria-label={mode === "tasks" ? "Zadania według statusu" : "Tematy według statusu"}>
          {(columns as Status[]).map(renderColumn)}
        </div>
      )}
      {menu && (
        <Menu
          anchor={menu.anchor}
          label="Przenieś do kolumny"
          items={menu.items}
          onClose={() => setMenu(null)}
          minWidth={190}
        />
      )}
    </div>
  );
}

function focusNeighbor(from: HTMLElement, dir: "up" | "down" | "left" | "right") {
  const col = from.closest<HTMLElement>("[data-col]");
  if (!col) return;
  const cards = Array.from(col.querySelectorAll<HTMLElement>("[data-card-id]"));
  const i = cards.indexOf(from);
  if (dir === "up" || dir === "down") {
    cards[i + (dir === "down" ? 1 : -1)]?.focus();
    return;
  }
  const cols = Array.from(col.parentElement?.querySelectorAll<HTMLElement>("[data-col]") ?? []);
  let c = cols.indexOf(col) + (dir === "right" ? 1 : -1);
  for (; c >= 0 && c < cols.length; c += dir === "right" ? 1 : -1) {
    const there = cols[c]!.querySelectorAll<HTMLElement>("[data-card-id]");
    if (there.length) {
      there[Math.min(i, there.length - 1)]?.focus();
      return;
    }
  }
}

interface ShellProps {
  id: string;
  col: string;
  dragging: boolean;
  onOpen: () => void;
  onPointerDown: (e: PointerEvent<HTMLElement>) => void;
  onKeyDown: (e: KeyboardEvent<HTMLElement>) => void;
}

function Card({
  id,
  col,
  dragging,
  onOpen,
  onPointerDown,
  onKeyDown,
  label,
  children,
}: ShellProps & { label: string; children: ReactNode }) {
  return (
    <article
      className={`card${dragging ? " dragging" : ""}`}
      tabIndex={0}
      data-card-id={id}
      data-card-col={col}
      aria-label={label}
      aria-roledescription="karta"
      onClick={onOpen}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
    >
      {children}
    </article>
  );
}

function TaskCard({ task, showProject, ...shell }: ShellProps & { task: Task; showProject: boolean }) {
  const view = useView();
  const ctx = taskContext(view.snapshot, task.location);
  const overdue = task.status !== "done" && isOverdue(task.due, view.today);
  const where = [showProject ? ctx.project?.name : null, ctx.topic?.title].filter(Boolean).join(" · ") || ctx.project?.name || "";
  return (
    <Card {...shell} label={task.title}>
      <div className="card-title">
        {task.priority === "high" && (
          <span className="prio-mark" title="Wysoki priorytet" aria-label="Wysoki priorytet">
            !
          </span>
        )}
        {task.title}
      </div>
      {(task.due || task.links.length > 0 || task.priority === "low") && (
        <div className="card-meta">
          {task.due && (
            <span className={`due${overdue ? " overdue" : ""}`} title={`Termin: ${task.due.replace("T", " ")}`}>
              {formatDue(task.due, view.today)}
            </span>
          )}
          {task.priority === "low" && <ArrowDown size={12} className="meta-icon" aria-label="Niski priorytet" />}
          {task.links[0] && <LinkChip url={task.links[0]} />}
          {task.links.length > 1 && <span className="meta-more">+{task.links.length - 1}</span>}
        </div>
      )}
      <div className="card-ctx" title={`${ctx.project?.name ?? ""}${ctx.topic ? ` · ${ctx.topic.title}` : ""}`}>
        <ProjectDot color={ctx.project?.color} size={6} />
        <span className="ctx-text">{where}</span>
      </div>
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
  const line = firstLine(topic.summary);
  return (
    <Card {...shell} label={topic.title}>
      <div className="card-title">{topic.title}</div>
      {line && <p className="card-summary">{line}</p>}
      <div className="card-foot">
        <span className="card-ctx" title={project?.name}>
          <ProjectDot color={project?.color} size={6} />
          {showProject && <span className="ctx-text">{project?.name ?? topic.project}</span>}
        </span>
        <span className="count" title={`${open} otwartych z ${total} zadań`}>
          {open}/{total}
        </span>
        {topic.updated && (
          <span className="age" title={`Zaktualizowano ${topic.updated.replace("T", " ")}`}>
            {formatAge(topic.updated, view.today)}
          </span>
        )}
      </div>
    </Card>
  );
}

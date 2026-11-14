import { AlignLeft, ArrowDown, FolderInput } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";

import type { Task, TaskStatus } from "../api";
import { formatDue, isOverdue } from "../lib/dates";
import { titleCommit } from "../lib/form";
import { moveTargets, sameLocation, taskContext, taskDraftKey } from "../lib/group";
import { TASK_STATUSES, TASK_STATUS_KEY, TASK_STATUS_LABEL } from "../lib/labels";
import { linkLabel } from "../lib/text";
import { links as routes } from "../router/router";
import { useDraft } from "../store/drafts";
import { takeReveal, useReveal, useTaskFlag } from "../store/ui";
import { useView } from "../store/vault";
import { useActions } from "./actions";
import { Menu, type MenuAnchor, type MenuItem } from "./Menu";
import { StatusToggle } from "./StatusToggle";
import { TaskDetail } from "./TaskDetail";
import { IconButton, ProjectDot } from "./ui";

const KEY_TO_STATUS: Record<string, TaskStatus> = Object.fromEntries(
  TASK_STATUSES.map((s) => [TASK_STATUS_KEY[s], s]),
) as Record<string, TaskStatus>;

function focusSibling(from: HTMLElement, dir: 1 | -1) {
  const rows = Array.from(document.querySelectorAll<HTMLElement>("[data-task-row]"));
  const i = rows.indexOf(from);
  rows[i + dir]?.focus();
}

export function LinkChip({ url, onRemove }: { url: string; onRemove?: () => void }) {
  const { openUrl } = useActions();
  return (
    <span className="chip">
      <button
        type="button"
        className="chip-link"
        title={url}
        onClick={(e) => {
          e.stopPropagation();
          void openUrl(url);
        }}
      >
        {linkLabel(url)}
      </button>
      {onRemove && (
        <button type="button" className="chip-x" aria-label={`Usuń link ${url}`} onClick={onRemove}>
          ×
        </button>
      )}
    </span>
  );
}

/** "Move to…" menu items for a task. */
export function useMoveItems(task: Task): MenuItem[] {
  const view = useView();
  const { moveTask } = useActions();
  const items: MenuItem[] = [];
  let group = "";
  for (const t of moveTargets(view.snapshot, task.location)) {
    if (t.group && t.group !== group) {
      group = t.group;
      items.push({ key: `h:${group}`, label: group, header: true });
    }
    items.push({
      key: t.key,
      label: t.group ? (t.location.kind === "project" ? "Ogólne zadania projektu" : t.label) : t.label,
      indent: !!t.group,
      checked: sameLocation(t.location, task.location),
      onSelect: () => void moveTask(task, t.location),
    });
  }
  return items;
}

export function TaskRow({ task, showContext = false }: { task: Task; showContext?: boolean }) {
  const view = useView();
  const actions = useActions();
  // Keyed by task id in a module store, so the detail stays open when the task
  // moves to another list (and when the file changes under it).
  const [sharedExpanded, setSharedExpanded] = useTaskFlag("expanded", task.id);
  const [localExpanded, setLocalExpanded] = useState(false);
  const expanded = task.id ? sharedExpanded : localExpanded;
  const setExpanded = (v: boolean | ((x: boolean) => boolean)) => {
    const next = typeof v === "function" ? v(expanded) : v;
    if (task.id) setSharedExpanded(next);
    else setLocalExpanded(next);
  };
  const [editing, setEditing] = useTaskFlag("renaming", task.id);
  const reveal = useReveal();
  const [flash, setFlash] = useState(false);
  // The same draft as the detail panel's title field (one save in flight per field).
  const titleDraft = useDraft(`${taskDraftKey(task)}:title`, task.title);
  const [askDelete, setAskDelete] = useState(false);
  const [menu, setMenu] = useState<{ kind: "status" | "move"; anchor: MenuAnchor } | null>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  const moveItems = useMoveItems(task);

  const ctx = taskContext(view.snapshot, task.location);
  const accent = ctx.project?.color ?? undefined;
  const overdue = task.status !== "done" && isOverdue(task.due, view.today);
  const disabled = !task.id;

  const closeMenu = useCallback(() => setMenu(null), []);

  // Jumped here from the board / palette: scroll to the task and flash it.
  useEffect(() => {
    if (!task.id || reveal.task !== task.id) return;
    const t = window.setTimeout(() => {
      if (!takeReveal(reveal, `row:${task.id}`)) return;
      rowRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
      setFlash(true);
    }, 60);
    const off = window.setTimeout(() => setFlash(false), 1800);
    return () => {
      window.clearTimeout(t);
      window.clearTimeout(off);
    };
  }, [reveal.nonce, reveal.task, task.id]);

  const statusItems: MenuItem[] = TASK_STATUSES.map((s) => ({
    key: s,
    label: TASK_STATUS_LABEL[s],
    hint: TASK_STATUS_KEY[s],
    checked: s === task.status,
    onSelect: () => void actions.setStatus(task, s),
  }));

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    if (k === "Enter") setExpanded((x) => !x);
    else if (k === " " || k === "x") actions.toggleDone(task);
    else if (KEY_TO_STATUS[k]) actions.setStatus(task, KEY_TO_STATUS[k]!);
    else if (k === "e" || k === "F2") setEditing(true);
    else if (k === "s") setMenu({ kind: "status", anchor: e.currentTarget });
    else if (k === "m") setMenu({ kind: "move", anchor: e.currentTarget });
    else if (k === "Delete" || k === "Backspace") {
      setExpanded(true);
      setAskDelete(true);
    } else if (k === "Escape") setExpanded(false);
    else if (k === "ArrowDown" || k === "j") focusSibling(e.currentTarget, 1);
    else if (k === "ArrowUp" || k === "k") focusSibling(e.currentTarget, -1);
    else return;
    e.preventDefault();
  };

  // Enter / Escape end the edit; the blur that follows must not save a second time.
  const settled = useRef(false);
  useEffect(() => {
    if (editing) settled.current = false;
  }, [editing]);

  // Saves only a title the user changed, judged against what will be stored
  // (an agent may have renamed the task while the field was open; a previous
  // rename may still be in flight, and a change back must then be saved too).
  const commitTitle = () => {
    if (settled.current) return;
    settled.current = true;
    setEditing(false);
    const commit = titleCommit(titleDraft.leave());
    if (commit.kind === "save") {
      const v = commit.value;
      titleDraft.save(v, () => actions.updateTask(task, { title: v }), (t) => t.title);
    } else if (commit.kind === "revert") titleDraft.cancelEdit(false);
    rowRef.current?.focus();
  };

  const contextLabel = ctx.project
    ? ctx.topic
      ? `${ctx.project.name} · ${ctx.topic.title}`
      : ctx.project.name
    : "Skrzynka";

  return (
    <div
      className={`task${expanded ? " expanded" : ""}${flash ? " flash" : ""}`}
      data-status={task.status}
      style={accent ? ({ "--accent": accent } as CSSProperties) : undefined}
    >
      <div
        ref={rowRef}
        className="task-row"
        tabIndex={0}
        data-task-row
        aria-expanded={expanded}
        aria-label={task.title}
        onKeyDown={onKeyDown}
        onClick={() => !editing && setExpanded((x) => !x)}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ kind: "status", anchor: { x: e.clientX, y: e.clientY } });
        }}
      >
        <StatusToggle
          status={task.status}
          disabled={disabled}
          onToggle={() => void actions.toggleDone(task)}
          onMenu={(anchor) => setMenu({ kind: "status", anchor })}
        />
        {task.priority === "high" && (
          <span className="prio-high" title="Wysoki priorytet" aria-label="Wysoki priorytet">
            !
          </span>
        )}
        {editing ? (
          <input
            className="title-edit"
            autoFocus
            value={titleDraft.draft}
            aria-label="Tytuł zadania"
            title={titleDraft.elsewhere ? "Ten tytuł zmienił się poza aplikacją — zapis zastosuje Twoją wersję." : undefined}
            onChange={(e) => titleDraft.setDraft(e.target.value)}
            onFocus={titleDraft.onFocus}
            onClick={(e) => e.stopPropagation()}
            onBlur={commitTitle}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") commitTitle();
              if (e.key === "Escape") {
                settled.current = true;
                titleDraft.cancelEdit(false);
                setEditing(false);
                rowRef.current?.focus();
              }
            }}
          />
        ) : (
          <span
            className="task-title"
            title={task.title}
            onDoubleClick={(e) => {
              e.stopPropagation();
              if (!disabled) setEditing(true);
            }}
          >
            {task.title}
          </span>
        )}
        <span className="task-meta">
          {task.notes && !expanded && <AlignLeft size={13} className="meta-icon" aria-label="Ma notatki" />}
          {!expanded && task.links.slice(0, 2).map((l) => <LinkChip key={l} url={l} />)}
          {!expanded && task.links.length > 2 && <span className="meta-more">+{task.links.length - 2}</span>}
          {showContext && (
            <a
              className="ctx"
              href={ctx.project ? routes.project(ctx.project.slug, ctx.topic?.slug) : routes.inbox()}
              onClick={(e) => e.stopPropagation()}
              title={contextLabel}
            >
              {ctx.project && <ProjectDot color={ctx.project.color} size={6} />}
              <span className="ctx-text">{contextLabel}</span>
            </a>
          )}
          {task.due && (
            <span className={`due${overdue ? " overdue" : ""}`} title={`Termin: ${task.due.replace("T", " ")}`}>
              {formatDue(task.due, view.today)}
            </span>
          )}
          {task.priority === "low" && <ArrowDown size={12} className="meta-icon" aria-label="Niski priorytet" />}
        </span>
        <span className="row-actions">
          <IconButton
            label="Przenieś do…"
            disabled={disabled}
            onClick={(e) => {
              e.stopPropagation();
              setMenu({ kind: "move", anchor: e.currentTarget });
            }}
          >
            <FolderInput size={14} />
          </IconButton>
        </span>
      </div>
      {expanded && (
        <TaskDetail
          task={task}
          askDelete={askDelete}
          onAskDeleteDone={() => setAskDelete(false)}
          onMove={(anchor) => setMenu({ kind: "move", anchor })}
        />
      )}
      {menu && (
        <Menu
          anchor={menu.anchor}
          label={menu.kind === "status" ? "Status zadania" : "Przenieś do"}
          items={menu.kind === "status" ? statusItems : moveItems}
          onClose={closeMenu}
          minWidth={menu.kind === "move" ? 260 : 200}
        />
      )}
    </div>
  );
}

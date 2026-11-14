import { FileText, FolderInput } from "lucide-react";
import { useRef, type FocusEvent } from "react";

import type { Priority, Task, TaskStatus } from "../api";
import { addDays } from "../lib/dates";
import { deadlineParts, deadlineToSave, fromDue, withDate, withTime } from "../lib/deadline";
import { titleCommit } from "../lib/form";
import { locationPath, taskContext, taskDraftKey } from "../lib/group";
import { PRIORITIES, PRIORITY_LABEL, TASK_STATUSES, TASK_STATUS_KEY, TASK_STATUS_LABEL } from "../lib/labels";
import { focusStaysWith, useDraft } from "../store/drafts";
import { useVault, useView } from "../store/vault";
import { useActions } from "./actions";
import { LinkChip } from "./TaskRow";
import { Button, ConfirmButton, DraftHint, Segmented, SubmitInput } from "./ui";

const URL_RE = /^[a-z][a-z0-9+.-]*:\/\/\S+$/i;

export function TaskDetail({
  task,
  askDelete,
  onAskDeleteDone,
  onMove,
}: {
  task: Task;
  askDelete: boolean;
  onAskDeleteDone: () => void;
  onMove: (anchor: HTMLElement) => void;
}) {
  const view = useView();
  const actions = useActions();
  const { notify } = useVault();
  // Drafts survive file changes on disk and remounts (see store/drafts.ts).
  // Leaving a field saves only what the user changed, judged at that moment
  // against what will be stored once a save in flight lands; a save is
  // acknowledged by its own result, and a field has at most one save in flight
  // (a newer value waits for it). Keyed by the task (lib/group.ts), so the
  // title draft is shared with the row's inline rename.
  const dk = `${taskDraftKey(task)}:`;
  const titleD = useDraft(`${dk}title`, task.title);
  const notesD = useDraft(`${dk}notes`, task.notes);
  // Date + optional time are one atomic draft (lib/deadline.ts).
  const dueD = useDraft(`${dk}due`, fromDue(task.due));
  const dateRef = useRef<HTMLInputElement>(null);
  const timeRef = useRef<HTMLInputElement>(null);
  const title = titleD.draft;
  const notes = notesD.draft;
  const { date: dueDate, time: dueTime } = deadlineParts(dueD.draft);
  const disabled = !task.id;
  const ctx = taskContext(view.snapshot, task.location);
  const where = ctx.project ? (ctx.topic ? `${ctx.project.name} · ${ctx.topic.title}` : `${ctx.project.name} · ogólne`) : "Skrzynka";
  const ids = { title: `title-${task.id}`, notes: `notes-${task.id}`, date: `due-date-${task.id}`, time: `due-time-${task.id}` };

  // No shortcut against `task.title` & co.: the snapshot may still show the
  // value a pending save is replacing, and a change back to it must be saved.
  const saveTitle = () => {
    const commit = titleCommit(titleD.leave());
    if (commit.kind === "save") {
      const v = commit.value;
      titleD.save(v, () => actions.updateTask(task, { title: v }), (t) => t.title);
    } else if (commit.kind === "revert") titleD.cancelEdit();
  };

  const saveNotes = () => {
    const edited = notesD.leave();
    if (edited !== null) notesD.save(edited, () => actions.updateTask(task, { notes: edited }), (t) => t.notes);
  };

  const saveDeadline = (draft: string) => {
    const next = deadlineToSave(draft);
    dueD.save(fromDue(next), () => actions.updateTask(task, { due: next }), (t) => fromDue(t.due));
  };
  const leaveDeadline = () => {
    const edited = dueD.leave();
    if (edited !== null) saveDeadline(edited);
  };
  /** "Dziś" / "Jutro" / "Bez terminu": an explicit new deadline. */
  const setDeadline = (draft: string) => {
    if (disabled) return;
    dueD.setDraft(draft);
    saveDeadline(draft);
  };
  // Moving between the date and the time input is not leaving the deadline.
  const onDeadlineBlur = (e: FocusEvent) => {
    if (!focusStaysWith(e, dueD, [dateRef.current, timeRef.current])) leaveDeadline();
  };
  // Buttons next to a field must not blur it first (that would autosave the draft).
  const keepFocus = (e: React.MouseEvent) => e.preventDefault();

  return (
    <div className="detail" onKeyDown={(e) => e.key === "Escape" && e.stopPropagation()}>
      <label className="d-label" htmlFor={ids.title}>
        Tytuł
      </label>
      <input
        id={ids.title}
        className="field"
        value={title}
        disabled={disabled}
        onChange={(e) => titleD.setDraft(e.target.value)}
        onFocus={titleD.onFocus}
        onBlur={(e) => !focusStaysWith(e, titleD) && saveTitle()}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
      <DraftHint draft={titleD} what="Tytuł" onLeave={saveTitle} fields={[ids.title]} />

      <span className="d-label">Status</span>
      <Segmented<TaskStatus>
        label="Status"
        value={task.status}
        disabled={disabled}
        options={TASK_STATUSES.map((s) => ({ value: s, label: TASK_STATUS_LABEL[s], hint: `Skrót: ${TASK_STATUS_KEY[s]}` }))}
        onChange={(s) => void actions.setStatus(task, s)}
      />

      <span className="d-label">Priorytet</span>
      <Segmented<Priority>
        label="Priorytet"
        value={task.priority}
        disabled={disabled}
        options={PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }))}
        onChange={(p) => void actions.updateTask(task, { priority: p })}
      />

      <span className="d-label">Termin</span>
      <div className="d-row">
        <input
          ref={dateRef}
          id={ids.date}
          type="date"
          className="field field-date"
          aria-label="Data"
          value={dueDate}
          disabled={disabled}
          onChange={(e) => dueD.setDraft(withDate(dueD.draft, e.target.value))}
          onFocus={dueD.onFocus}
          onBlur={onDeadlineBlur}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
        <input
          ref={timeRef}
          id={ids.time}
          type="time"
          className="field field-time"
          aria-label="Godzina (opcjonalnie)"
          value={dueTime}
          disabled={disabled || !dueDate}
          onChange={(e) => dueD.setDraft(withTime(dueD.draft, e.target.value))}
          onFocus={dueD.onFocus}
          onBlur={onDeadlineBlur}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
        <Button variant="quiet" disabled={disabled} onMouseDown={keepFocus} onClick={() => setDeadline(withDate(dueD.draft, view.today))}>
          Dziś
        </Button>
        <Button variant="quiet" disabled={disabled} onMouseDown={keepFocus} onClick={() => setDeadline(withDate(dueD.draft, addDays(view.today, 1)))}>
          Jutro
        </Button>
        {task.due && (
          <Button variant="quiet" disabled={disabled} onMouseDown={keepFocus} onClick={() => setDeadline("")}>
            Bez terminu
          </Button>
        )}
      </div>
      <DraftHint draft={dueD} what="Termin" onLeave={leaveDeadline} fields={[ids.date, ids.time]} />

      <span className="d-label">Linki</span>
      <div className="d-links">
        {task.links.map((l) => (
          <LinkChip key={l} url={l} onRemove={disabled ? undefined : () => void actions.updateTask(task, { remove_links: [l] })} />
        ))}
        <SubmitInput
          draftKey={`${dk}link`}
          className="field-inline"
          placeholder="Wklej link i naciśnij Enter"
          aria-label="Dodaj link"
          disabled={disabled}
          onSubmit={async (v) => {
            if (!URL_RE.test(v)) {
              notify("To nie wygląda na link — wklej pełny adres, np. https://…");
              return false;
            }
            return (await actions.updateTask(task, { add_links: [v] })) !== undefined;
          }}
        />
      </div>

      <label className="d-label" htmlFor={ids.notes}>
        Notatki
      </label>
      <textarea
        id={ids.notes}
        className="field notes"
        rows={Math.min(8, Math.max(2, notes.split("\n").length + 1))}
        placeholder="Szczegóły, kontekst, kroki…"
        value={notes}
        disabled={disabled}
        onChange={(e) => notesD.setDraft(e.target.value)}
        onFocus={notesD.onFocus}
        onBlur={(e) => !focusStaysWith(e, notesD) && saveNotes()}
        onKeyDown={(e) => (e.metaKey || e.ctrlKey) && e.key === "Enter" && e.currentTarget.blur()}
      />
      <DraftHint draft={notesD} what="Notatki" onLeave={saveNotes} fields={[ids.notes]} />

      <span className="d-label">Miejsce</span>
      <div className="d-row">
        <Button variant="ghost" disabled={disabled} onClick={(e) => onMove(e.currentTarget)}>
          <FolderInput size={13} /> {where}
        </Button>
      </div>

      <div className="d-footer">
        <span className="task-id" title="Identyfikator zadania w pliku">
          {task.id ? `^${task.id}` : "bez identyfikatora"}
        </span>
        <Button variant="quiet" onClick={() => void actions.reveal(locationPath(task.location))}>
          <FileText size={13} /> Pokaż plik
        </Button>
        <span className="spacer" />
        {!disabled && (
          <ConfirmButton
            question="Usunąć to zadanie?"
            confirmLabel="Usuń"
            autoOpen={askDelete}
            onCancel={onAskDeleteDone}
            onConfirm={() => void actions.deleteTask(task)}
          >
            Usuń zadanie
          </ConfirmButton>
        )}
      </div>
    </div>
  );
}

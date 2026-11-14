import { ChevronRight } from "lucide-react";
import { useEffect, type ReactNode } from "react";

import type { Task, TaskLocation } from "../api";
import { splitDone } from "../lib/group";
import { useReveal } from "../store/ui";
import { useActions } from "./actions";
import { TaskRow } from "./TaskRow";
import { SubmitInput, useStoredFlag } from "./ui";

/**
 * Open tasks, an optional quick-add input, and done tasks behind a per-list
 * toggle (remembered per list in this browser profile).
 */
export function TaskList({
  listId,
  tasks,
  addTo,
  addPlaceholder = "Dodaj zadanie",
  showContext = false,
  empty,
  autoFocusAdd = false,
  addFirst = false,
}: {
  listId: string;
  tasks: Task[];
  addTo?: TaskLocation;
  addPlaceholder?: string;
  showContext?: boolean;
  empty?: ReactNode;
  autoFocusAdd?: boolean;
  /** Put the quick-add input above the tasks (inbox). */
  addFirst?: boolean;
}) {
  const { captureTask } = useActions();
  const { open, done } = splitDone(tasks);
  const [showDone, setShowDone] = useStoredFlag(`montybrain.done.${listId}`, false);
  const reveal = useReveal();
  // A revealed task that is done must not stay hidden in the collapsed group.
  useEffect(() => {
    if (reveal.task && !showDone && done.some((t) => t.id === reveal.task)) setShowDone(true);
  }, [reveal.nonce]);

  const addRow = addTo && (
        <div className="add-row">
          <span className="add-plus" aria-hidden>
            +
          </span>
          <SubmitInput
            className="field-inline"
            placeholder={`${addPlaceholder}…`}
            title="Enter dodaje. Na końcu możesz dopisać !wysoki, !niski, @dziś, @jutro, @pt, @+3d albo @2026-11-20"
            aria-label={addPlaceholder}
            draftKey={`add:${listId}`}
            autoFocus={autoFocusAdd}
            onSubmit={async (v) => {
              const created = await captureTask(addTo, v);
              return created !== undefined;
            }}
          />
        </div>);

  return (
    <div className="task-list">
      {addFirst && addRow}
      {open.length === 0 && empty}
      {open.map((t) => (
        <TaskRow key={t.id ?? `${t.location.kind}:${t.line}`} task={t} showContext={showContext} />
      ))}
      {!addFirst && addRow}
      {done.length > 0 && (
        <div className="done-group">
          <button type="button" className="done-toggle" aria-expanded={showDone} onClick={() => setShowDone(!showDone)}>
            <ChevronRight size={12} className="chev" />
            Zrobione <span className="count">{done.length}</span>
          </button>
          {showDone &&
            done.map((t) => <TaskRow key={t.id ?? `${t.location.kind}:${t.line}`} task={t} showContext={showContext} />)}
        </div>
      )}
    </div>
  );
}

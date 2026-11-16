import { t as tr } from "../i18n";
import type { MontyBrainApi } from "../api/api";
import type { Task, TaskLocation, TaskStatus } from "../api/types";
import { describeError } from "./errors";
import { sameLocation } from "./group";

export interface TaskDrop {
  id: string;
  from: TaskLocation;
  to: TaskLocation;
  status: TaskStatus;
  changesLocation: boolean;
}

/** Empty column space changes status; a concrete topic/card also supplies a place. */
export function taskDrop(task: Task, status: TaskStatus, location: TaskLocation | null): TaskDrop | null {
  if (!task.id) return null;
  const to = location ?? task.location;
  const changesLocation = !sameLocation(task.location, to);
  if (!changesLocation && task.status === status) return null;
  return { id: task.id, from: task.location, to, status, changesLocation };
}

/** Called only after the user accepts a location change. A failed second write
 * restores the original place, and an external move invalidates the confirmation.
 */
export async function applyTaskDrop(api: Pick<MontyBrainApi, "getState" | "moveTask" | "updateTask">, drop: TaskDrop): Promise<Task> {
  const current = (await api.getState()).snapshot.tasks.find((t) => t.id === drop.id);
  if (!current) throw new Error(tr("Tego zadania już nie ma. Odśwież tablicę i wybierz inne."));
  if (drop.changesLocation && !sameLocation(current.location, drop.from)) {
    throw new Error(tr("Zadanie zmieniło temat w międzyczasie. Przeciągnij je ponownie, żeby potwierdzić nowe miejsce."));
  }
  let moved = current;
  if (drop.changesLocation) moved = await api.moveTask(drop.id, drop.to);
  if (current.status === drop.status) return moved;
  try {
    return await api.updateTask(drop.id, { status: drop.status });
  } catch (error) {
    if (drop.changesLocation) {
      try {
        await api.moveTask(drop.id, current.location);
      } catch {
        throw new Error(tr("Zadanie jest już w nowym miejscu, ale nie udało się zmienić statusu ani przywrócić poprzedniego tematu. {0}", describeError(error)));
      }
    }
    throw error;
  }
}

/** Compensate a card's layout shift without letting focus scroll it a second time. */
export function anchoredScrollTop(scrollTop: number, cardTop: number, wantedTop: number, maxScroll: number): number {
  return Math.max(0, Math.min(maxScroll, scrollTop + cardTop - wantedTop));
}

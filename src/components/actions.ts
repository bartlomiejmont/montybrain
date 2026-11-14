// Mutation helpers shared by the views. All go through the store's `run`, which
// re-fetches the state afterwards and reports errors as toasts.

import { useMemo } from "react";

import type { NewTask, Task, TaskLocation, TaskPatch, TaskStatus } from "../api";
import { useVault } from "../store/vault";

export function useActions() {
  const { run, notify } = useVault();
  return useMemo(() => {
    const withId = <T,>(task: Task, f: (id: string) => Promise<T | undefined>) => {
      if (!task.id) {
        notify("To zadanie nie ma jeszcze identyfikatora — za chwilę zostanie nadany.");
        return Promise.resolve(undefined);
      }
      return f(task.id);
    };
    return {
      updateTask: (task: Task, patch: TaskPatch) => withId(task, (id) => run((a) => a.updateTask(id, patch))),
      setStatus: (task: Task, status: TaskStatus) =>
        task.status === status ? Promise.resolve(undefined) : withId(task, (id) => run((a) => a.updateTask(id, { status }))),
      toggleDone: (task: Task) =>
        withId(task, (id) => run((a) => a.updateTask(id, { status: task.status === "done" ? "todo" : "done" }))),
      moveTask: (task: Task, to: TaskLocation) => withId(task, (id) => run((a) => a.moveTask(id, to))),
      deleteTask: (task: Task) => withId(task, (id) => run((a) => a.deleteTask(id))),
      createTask: (location: TaskLocation, input: NewTask) => run((a) => a.createTask(location, input)),
      /** A "dodaj zadanie" field: the text is read by the backend (title + trailing `!`/`@`). */
      captureTask: (location: TaskLocation, text: string) => run((a) => a.captureTask(location, text)),
      openUrl: (url: string) => run((a) => a.openUrl(url)),
      reveal: (path: string) => run((a) => a.revealPath(path)),
    };
  }, [run, notify]);
}

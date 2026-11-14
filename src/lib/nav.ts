// Hash links to items (task → its container view with the task revealed).

import type { Snapshot, Task, Topic } from "../api/types";
import { href } from "../router/registry";

export const boardHref = () => href("tablica");

export function taskHref(task: Task): string {
  const loc = task.location;
  const zadanie = task.id ?? undefined;
  if (loc.kind === "inbox") return href("skrzynka", {}, { zadanie });
  return href("p/:project", { project: loc.project }, { temat: loc.kind === "topic" ? loc.topic : undefined, zadanie });
}

export function topicHref(topic: Pick<Topic, "project" | "slug">): string {
  return href("p/:project", { project: topic.project }, { temat: topic.slug });
}

export function projectOf(snap: Snapshot, slug: string) {
  return snap.projects.find((p) => p.slug === slug) ?? null;
}

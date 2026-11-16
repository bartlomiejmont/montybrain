import type { Snapshot, Task, Topic } from "../api";
import { taskDraftKey } from "./group";

export type DetailTarget =
  | { kind: "task"; key: string }
  | { kind: "topic"; project: string; topic: string }
  | { kind: "project"; project: string };

export const taskTarget = (task: Task): DetailTarget => ({ kind: "task", key: taskDraftKey(task) });
export const topicTarget = (topic: Pick<Topic, "project" | "slug">): DetailTarget => ({ kind: "topic", project: topic.project, topic: topic.slug });

/** Resolve on every refresh: a panel must never edit an old copy of an item. */
export function resolveDetail(snapshot: Snapshot, target: DetailTarget) {
  const task = target.kind === "task" ? snapshot.tasks.find((t) => taskDraftKey(t) === target.key) : undefined;
  const location = task?.location;
  const topic = target.kind === "topic"
    ? snapshot.topics.find((t) => t.project === target.project && t.slug === target.topic)
    : location?.kind === "topic"
      ? snapshot.topics.find((t) => t.project === location.project && t.slug === location.topic)
      : undefined;
  const projectSlug = target.kind !== "task" ? target.project : location?.kind !== "inbox" ? location?.project : undefined;
  const project = snapshot.projects.find((p) => p.slug === projectSlug);
  const exists = target.kind === "task" ? !!task : target.kind === "topic" ? !!topic : !!project;
  return { task, topic, project, exists };
}

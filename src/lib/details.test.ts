import { describe, expect, it } from "vitest";

import { buildFixtures } from "../api/fixtures";
import { resolveDetail, taskTarget, topicTarget } from "./details";

const snapshot = () => buildFixtures("2026-11-15", "2026-11-15T12:00");

describe("overview detail selection", () => {
  it("follows the same task after edits and a move, using its current topic", () => {
    const snap = snapshot();
    const task = snap.tasks.find((t) => t.id && t.location.kind === "topic")!;
    const target = taskTarget(task);
    const destination = snap.topics.find((t) => t.project === "montybrain")!;
    const nextTask = { ...task, title: "Zmieniony tytuł", notes: "Nowy opis", status: "doing" as const, location: { kind: "topic" as const, project: destination.project, topic: destination.slug } };
    const updated = { ...snap, tasks: snap.tasks.map((t) => t === task ? nextTask : t) };
    const detail = resolveDetail(updated, target);
    expect(detail.task).toBe(nextTask);
    expect(detail.topic).toBe(destination);
    expect(detail.project?.slug).toBe("montybrain");
    expect(detail.exists).toBe(true);
  });

  it("keeps a finished topic selected even when it leaves the active overview", () => {
    const snap = snapshot();
    const topic = snap.topics.find((t) => t.status === "in_progress")!;
    const finished = { ...topic, status: "done" as const, summary: "Zakończone." };
    const next = { ...snap, topics: snap.topics.map((t) => t === topic ? finished : t) };
    expect(resolveDetail(next, topicTarget(topic))).toMatchObject({ exists: true, topic: finished });
  });

  it("distinguishes identical topic slugs in different projects", () => {
    const snap = snapshot();
    const topic = snap.topics[0]!;
    const other = { ...topic, project: "montybrain", title: "Inny temat" };
    snap.topics.push(other);
    expect(resolveDetail(snap, topicTarget(other)).topic).toBe(other);
    expect(resolveDetail(snap, topicTarget(topic)).topic).toBe(topic);
  });

  it("reports deletion instead of displaying a stale editable copy", () => {
    const snap = snapshot();
    const task = snap.tasks.find((t) => t.id)!;
    const target = taskTarget(task);
    snap.tasks = snap.tasks.filter((t) => t !== task);
    expect(resolveDetail(snap, target)).toMatchObject({ exists: false, task: undefined, topic: undefined });
  });

  it("handles inbox and unidentified tasks without confusing their containers", () => {
    const snap = snapshot();
    const task = { ...snap.tasks[0]!, id: null, location: { kind: "inbox" as const } };
    snap.tasks.push(task);
    const detail = resolveDetail(snap, taskTarget(task));
    expect(detail).toEqual({ exists: true, task, topic: undefined, project: undefined });
  });
});

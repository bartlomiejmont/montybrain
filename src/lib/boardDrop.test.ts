import { describe, expect, it, vi } from "vitest";
import { createMockApi } from "../api/mock";
import type { TaskLocation } from "../api/types";
import { anchoredScrollTop, applyTaskDrop, taskDrop } from "./boardDrop";

const source: TaskLocation = { kind: "topic", project: "whenubuy", topic: "ab-testy-cen" };
const destination: TaskLocation = { kind: "topic", project: "montybrain", topic: "tablica" };
async function setup() {
  const api = createMockApi();
  const target = await api.createTopic("montybrain", { title: "Tablica" });
  const to: TaskLocation = { kind: "topic", project: "montybrain", topic: target.slug };
  const task = await api.createTask(source, { title: "Przenieść kartę", notes: "Opis zostaje", links: ["https://example.com"], due: "2026-12-20", priority: "high" });
  return { api, task, to };
}

describe("board task drops", () => {
  it("keeps the topic when dropped on a column header, and ignores an unchanged drop", async () => {
    const { task } = await setup();
    expect(taskDrop(task, "doing", null)).toMatchObject({ to: source, status: "doing", changesLocation: false });
    expect(taskDrop(task, task.status, source)).toBeNull();
    expect(taskDrop({ ...task, id: null }, "doing", destination)).toBeNull();
  });

  it("plans a different topic even in the same status, including another project", async () => {
    const { task, to } = await setup();
    expect(taskDrop(task, task.status, to)).toEqual({ id: task.id, from: source, to, status: task.status, changesLocation: true });
    const sameProject: TaskLocation = { kind: "topic", project: "whenubuy", topic: "onboarding" };
    expect(taskDrop(task, "doing", sameProject)).toMatchObject({ to: sameProject, status: "doing", changesLocation: true });
  });

  it("persists both destination and status while preserving task content and id", async () => {
    const { api, task, to } = await setup();
    await applyTaskDrop(api, taskDrop(task, "doing", to)!);
    const saved = (await api.getState()).snapshot.tasks.find((t) => t.id === task.id);
    expect(saved).toMatchObject({ id: task.id, title: task.title, notes: task.notes, links: task.links, due: task.due, priority: task.priority, location: to, status: "doing" });
  });

  it("moves to another topic without rewriting an unchanged status", async () => {
    const { api, task, to } = await setup();
    const update = vi.spyOn(api, "updateTask");
    await applyTaskDrop(api, taskDrop(task, task.status, to)!);
    expect(update).not.toHaveBeenCalled();
    expect((await api.getState()).snapshot.tasks.find((t) => t.id === task.id)?.location).toEqual(to);
  });

  it("does not move location for a status-only drop", async () => {
    const { api, task } = await setup();
    const move = vi.spyOn(api, "moveTask");
    expect(await applyTaskDrop(api, taskDrop(task, "doing", null)!)).toMatchObject({ status: "doing", location: source });
    expect(move).not.toHaveBeenCalled();
  });

  it("does not write status if moving to the destination fails", async () => {
    const { api, task, to } = await setup();
    vi.spyOn(api, "moveTask").mockRejectedValue(new Error("Brak dostępu"));
    const update = vi.spyOn(api, "updateTask");
    await expect(applyTaskDrop(api, taskDrop(task, "doing", to)!)).rejects.toThrow("Brak dostępu");
    expect(update).not.toHaveBeenCalled();
  });

  it("restores the original topic when the status write fails", async () => {
    const { api, task, to } = await setup();
    vi.spyOn(api, "updateTask").mockRejectedValue(new Error("Błąd zapisu"));
    await expect(applyTaskDrop(api, taskDrop(task, "doing", to)!)).rejects.toThrow("Błąd zapisu");
    expect((await api.getState()).snapshot.tasks.find((t) => t.id === task.id)).toMatchObject({ location: source, status: task.status, notes: task.notes });
  });

  it("reports the actual location if rollback also fails", async () => {
    const { api, task, to } = await setup();
    const originalMove = api.moveTask.bind(api);
    vi.spyOn(api, "moveTask").mockImplementationOnce(originalMove).mockRejectedValue(new Error("Nie można cofnąć"));
    vi.spyOn(api, "updateTask").mockRejectedValue(new Error("Błąd statusu"));
    await expect(applyTaskDrop(api, taskDrop(task, "doing", to)!)).rejects.toThrow("Zadanie jest już w nowym miejscu");
    expect((await api.getState()).snapshot.tasks.find((t) => t.id === task.id)?.location).toEqual(to);
  });

  it("invalidates confirmation if someone moved or deleted the task meanwhile", async () => {
    const { api, task, to } = await setup();
    const drop = taskDrop(task, "doing", to)!;
    await api.moveTask(task.id!, { kind: "project", project: "whenubuy" });
    const move = vi.spyOn(api, "moveTask");
    const update = vi.spyOn(api, "updateTask");
    await expect(applyTaskDrop(api, drop)).rejects.toThrow("zmieniło temat w międzyczasie");
    expect(move).not.toHaveBeenCalled();
    expect(update).not.toHaveBeenCalled();
    await api.deleteTask(task.id!);
    await expect(applyTaskDrop(api, drop)).rejects.toThrow("zadania już nie ma");
  });
});

describe("board viewport compensation", () => {
  it("keeps the card at the release height after rows above grow or shrink", () => {
    expect(anchoredScrollTop(400, 500, 300, 2000)).toBe(600);
    expect(anchoredScrollTop(600, 100, 300, 2000)).toBe(400);
    expect(anchoredScrollTop(400, 300, 300, 2000)).toBe(400);
  });
  it("respects the top and bottom of the scroll range", () => {
    expect(anchoredScrollTop(20, 100, 300, 1000)).toBe(0);
    expect(anchoredScrollTop(900, 500, 200, 1000)).toBe(1000);
  });
});

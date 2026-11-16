import { describe, expect, it, vi } from "vitest";

import { createMockApi } from "../api/mock";
import { createAndLinkBranch } from "./branchTriage";
import { buildToday } from "./today";

describe("actionable unassigned branches", () => {
  it("creates a topic, assigns the branch and removes its unassigned signal", async () => {
    const api = createMockApi();
    const onCreated = vi.fn();
    const result = await createAndLinkBranch(api, "whenubuy", "whenubuy-mobile", "feature/loyalty", "Program lojalnościowy", null, onCreated);
    expect(onCreated).toHaveBeenCalledWith(result.topic);
    const project = result.drift.find((d) => d.project === "whenubuy")!;
    expect(project.unknown.some((b) => b.branch === "feature/loyalty")).toBe(false);
    const state = await api.getState();
    expect(state.snapshot.topics.find((t) => t.project === "whenubuy" && t.slug === result.topic)).toMatchObject({ title: "Program lojalnościowy", repo: "whenubuy-mobile", status: "planned" });
  });

  it("retries linking after a failure without creating the same topic again", async () => {
    const api = createMockApi();
    const create = vi.spyOn(api, "createTopic");
    vi.spyOn(api, "linkBranch").mockRejectedValueOnce(new Error("Chwilowy błąd zapisu"));
    let created: string | null = null;
    const remember = (slug: string) => { created = slug; };
    await expect(createAndLinkBranch(api, "whenubuy", "whenubuy-mobile", "feature/loyalty", "Program lojalnościowy", created, remember)).rejects.toThrow("Chwilowy błąd zapisu");
    expect(created).toBeTruthy();
    const result = await createAndLinkBranch(api, "whenubuy", "whenubuy-mobile", "feature/loyalty", "Program lojalnościowy", created, remember);
    expect(create).toHaveBeenCalledTimes(1);
    expect(result.topic).toBe(created);
    expect(result.drift.find((d) => d.project === "whenubuy")!.unknown.some((b) => b.branch === "feature/loyalty")).toBe(false);
  });

  it("drops the project from today's attention once all its unassigned branches are handled", async () => {
    const api = createMockApi();
    const before = await api.getState();
    const sections = (snapshot: typeof before.snapshot, drift: Awaited<ReturnType<typeof api.listDrift>>) => buildToday({ snapshot, today: before.today, waiting: [], drift, updates: [] });
    expect(sections(before.snapshot, await api.listDrift()).attention.some((r) => r.kind === "project" && r.project === "whenubuy")).toBe(true);
    await api.linkBranch("whenubuy", "whenubuy-mobile", "feature/loyalty", "ab-testy-cen");
    const drift = await api.ignoreBranch("whenubuy", "whenubuy-backend", "spike/graphql");
    const after = await api.getState();
    expect(sections(after.snapshot, drift).attention.some((r) => r.kind === "project" && r.project === "whenubuy")).toBe(false);
  });
});

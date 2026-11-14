import { describe, expect, it, vi } from "vitest";

import { createMockApi } from "./mock";

describe("mock drift and waiting (browser states)", () => {
  it("links and ignores move branches out of the unknown list", async () => {
    const api = createMockApi();
    const d = (await api.listDrift()).find((x) => x.project === "whenubuy")!;
    expect(d.unknown.map((b) => b.branch)).toEqual(["feature/loyalty", "spike/graphql"]);
    expect(d.spread).toEqual([{ topic: "platnosci-stripe-przelewy24", branches: 3, repos: 2 }]);
    const linked = (await api.linkBranch("whenubuy", "whenubuy-mobile", "feature/loyalty", "ab-testy-cen")).find((x) => x.project === "whenubuy")!;
    expect(linked.unknown.map((b) => b.branch)).toEqual(["spike/graphql"]);
    const ignored = (await api.ignoreBranch("whenubuy", "whenubuy-backend", "spike/graphql")).find((x) => x.project === "whenubuy")!;
    expect(ignored.unknown).toEqual([]);
    await expect(api.linkBranch("whenubuy", "whenubuy-mobile", "x", "nie-ma")).rejects.toThrow(/nie ma takiego tematu/);
    expect((await createMockApi({ drift: "none" }).listDrift()).every((x) => x.collected_at === null)).toBe(true);
  });

  it("waiting items follow the vault; confirm and set move the next check", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 14, 10, 0));
    const api = createMockApi();
    let w = await api.listWaiting();
    const p24 = w.find((x) => x.key === "topic:whenubuy/platnosci-stripe-przelewy24")!;
    expect([p24.due, p24.status]).toEqual([true, "blocked"]);
    w = await api.confirmWaiting(p24.key);
    expect(w.find((x) => x.key === p24.key)!).toMatchObject({ due: false, next: "2026-11-16" });
    w = await api.setWaiting(p24.key, 7);
    expect(w.find((x) => x.key === p24.key)!.next).toBe("2026-11-21");
    await expect(api.setWaiting(p24.key, 5)).rejects.toThrow(/Nieprawidłowy odstęp/);
    // Leaving the waiting status clears the schedule; a reason comes from the journal.
    await api.updateTopic("whenubuy", "platnosci-stripe-przelewy24", { status: "in_progress" });
    expect((await api.listWaiting()).some((x) => x.key === p24.key)).toBe(false);
    await api.updateTopic("whenubuy", "ab-testy-cen", { status: "waiting" });
    await api.addJournalEntry("whenubuy", "ab-testy-cen", { author: "ja", text: "Czeka: dane z tygodnia" });
    const ab = (await api.listWaiting()).find((x) => x.key === "topic:whenubuy/ab-testy-cen")!;
    expect([ab.reason, ab.next]).toEqual(["dane z tygodnia", "2026-11-16"]);
    vi.useRealTimers();
  });

  it("a proposed new topic joins an existing one", async () => {
    const api = createMockApi({ updateScenario: "proposals" });
    const v = (await api.listUpdates()).find((x) => x.project === "whenubuy")!;
    const i = v.proposal!.changes.findIndex((c) => c.change.kind === "new_topic");
    // The proposal already has an entry for Płatności: joining there is refused.
    await expect(api.joinUpdate(v.proposal!.run_id, i, "platnosci-stripe-przelewy24")).rejects.toThrow(/popraw tamten wpis/);
    const r = await api.joinUpdate(v.proposal!.run_id, i, "ab-testy-cen");
    const p = r.updates.find((x) => x.project === "whenubuy")!.proposal!;
    expect(p.changes[i]!.change).toMatchObject({ kind: "topic_log", topic: "ab-testy-cen", status: null, summary: null });
    expect(p.changes[i]!.edited).toBe(true);
    expect(p.changes.slice(-2).map((c) => c.change.kind)).toEqual(["new_task", "new_task"]);
  });
});

import { describe, expect, it, vi } from "vitest";

import { createMockSession, mockSessionView } from "../api/mock-session";
import { createSessionStore } from "./session";

describe("agent session store", () => {
  it("starting opens the panel; events replace the view; hiding keeps the session", async () => {
    vi.useFakeTimers();
    const api = createMockSession(null);
    const s = createSessionStore(api);
    await s.start();
    expect(s.getSnapshot()).toMatchObject({ view: null, open: false });
    await s.begin({ scope: "inbox", provider: "claude", text: "Sprawdź Slacka" });
    expect(s.getSnapshot().open).toBe(true);
    expect(s.getSnapshot().view?.status).toBe("starting");
    await vi.advanceTimersByTimeAsync(2000);
    const v = s.getSnapshot().view!;
    expect(v.status).toBe("needs_you");
    s.hide();
    expect(s.getSnapshot().view?.status).toBe("needs_you");
    // Answering the card lets the session go on; the proposal arrives.
    const card = v.entries.find((e) => e.kind === "permission" && e.state === "pending")!;
    await s.answer(card.id, "allow_session");
    await vi.advanceTimersByTimeAsync(1000);
    expect(s.getSnapshot().view?.proposal?.count).toBe(3);
    expect(s.getSnapshot().view?.remembered).toEqual(["mcp__slack__read_channel"]);
    // A second answer to the same card is refused.
    await expect(s.answer(card.id, "allow")).rejects.toThrow();
    // A live session cannot be dismissed; an ended one can.
    await expect(s.dismiss()).rejects.toThrow();
    await s.stop();
    await s.dismiss();
    expect(s.getSnapshot()).toMatchObject({ view: null, open: false });
    vi.useRealTimers();
  });

  it("an older view of the same session does not replace a newer one", async () => {
    let emit: (v: ReturnType<typeof mockSessionView>) => void = () => {};
    const api = { ...createMockSession(null), onAgentSession: async (cb: typeof emit) => ((emit = cb), () => {}) };
    const s = createSessionStore(api);
    await s.start();
    const newer = mockSessionView("card");
    emit(newer);
    emit({ ...newer, entries: newer.entries.slice(0, 2) });
    expect(s.getSnapshot().view?.entries).toHaveLength(newer.entries.length);
    // A status change always wins.
    emit({ ...newer, status: "stopped", entries: newer.entries.slice(0, 2) });
    expect(s.getSnapshot().view?.status).toBe("stopped");
  });
});

describe("starting a contextual conversation", () => {
  it("keeps the active scope and the new draft when a second start is refused", async () => {
    const active = mockSessionView("working");
    const api = { ...createMockSession(active), agentSessionStart: vi.fn() };
    const s = createSessionStore(api);
    await s.start();
    const draft = { scope: "project" as const, project: "firma", topic: { slug: "umowy", title: "Umowy" }, text: "Sprawdź pocztę", origin: "#/mapa?focus=firma%2Fumowy" };
    s.prepare(draft);
    await expect(s.begin({ scope: "project", project: "firma", text: draft.text, provider: "codex" }, draft)).rejects.toThrow("Inna sesja");
    expect(api.agentSessionStart).not.toHaveBeenCalled();
    expect(s.getSnapshot().view?.scope).toEqual(active.scope);
    s.conversation();
    s.hide();
    s.compose({ scope: "inbox", text: "" });
    expect(s.getSnapshot().draft).toEqual(draft);
  });

  it("stops before replacement and preserves a failed request for retry", async () => {
    let current = mockSessionView("working");
    const calls: string[] = [];
    const api = {
      ...createMockSession(null),
      agentSessionGet: async () => current,
      agentSessionStop: async () => { calls.push("stop"); current = { ...current, status: "stopped" }; },
      agentSessionStart: async () => { calls.push("start"); throw new Error("CLI unavailable"); },
    };
    const s = createSessionStore(api);
    const draft = { scope: "project" as const, project: "firma", text: "Nowa prośba" };
    s.prepare(draft);
    await expect(s.begin({ ...draft, provider: "codex" }, draft, true)).rejects.toThrow("CLI unavailable");
    expect(calls).toEqual(["stop", "start"]);
    expect(s.getSnapshot()).toMatchObject({ draft, composing: true, busy: false });
    expect(s.getSnapshot().view?.status).toBe("stopped");
  });

  it("preserves context across navigation and prevents a double start", async () => {
    let finish!: (v: ReturnType<typeof mockSessionView>) => void;
    const api = { ...createMockSession(null), agentSessionStart: vi.fn(() => new Promise<ReturnType<typeof mockSessionView>>((resolve) => { finish = resolve; })) };
    const s = createSessionStore(api);
    await s.start();
    const draft = { scope: "project" as const, project: "firma", text: "Sprawdź", origin: "#/dzis", topic: { slug: "umowy", title: "Umowy" } };
    s.prepare(draft);
    const request = { ...draft, provider: "codex" as const };
    const pending = s.begin(request, draft);
    await expect(s.begin(request, draft)).rejects.toThrow("Uruchamianie");
    finish({ ...mockSessionView("working"), scope: { kind: "project", project: "firma" } });
    await pending;
    s.hide();
    s.open();
    expect(s.getSnapshot()).toMatchObject({ context: draft, draft: null, composing: false, open: true });
    expect(api.agentSessionStart).toHaveBeenCalledTimes(1);
  });
});

import { describe, expect, it, vi } from "vitest";

import { createMockSession, mockSessionView } from "../api/mock-session";
import { createSessionStore } from "./session";

describe("agent session store", () => {
  it("starting opens the panel; events replace the view; hiding keeps the session", async () => {
    vi.useFakeTimers();
    const api = createMockSession(null);
    const s = createSessionStore(api);
    await s.start();
    expect(s.getSnapshot()).toEqual({ view: null, open: false });
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
    expect(s.getSnapshot()).toEqual({ view: null, open: false });
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

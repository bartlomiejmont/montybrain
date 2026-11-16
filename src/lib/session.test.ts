import { describe, expect, it } from "vitest";

import { mockSessionView } from "../api/mock-session";
import type { SessionView } from "../api/types";
import { canInterrupt, canSend, cardActions, indicator, pendingCards, toolLabel, type Card } from "./session";

const card = (over: Partial<Card> = {}): Card => ({ kind: "permission", id: 1, title: "t", detail: "", tool: "Bash", state: "pending", can_session: true, note: "", questions: [], ...over });

describe("agent session panel", () => {
  it("a pending card has Zezwól / Zezwól na tę sesję / Odmów; nothing else has buttons", () => {
    expect(cardActions(card()).map((a) => a.answer)).toEqual(["allow", "allow_session", "deny"]);
    expect(cardActions(card({ can_session: false })).map((a) => a.answer)).toEqual(["allow", "deny"]);
    // What the app denied (MontyBrain writes, vault files) can never be approved from the window.
    for (const state of ["auto_denied", "denied", "allowed", "allowed_session", "auto_allowed", "withdrawn"] as const) {
      expect(cardActions(card({ state }))).toEqual([]);
    }
    // A question card answers with its options, not with Zezwól.
    expect(cardActions(card({ questions: [{ id: "k", header: "", question: "?", options: ["a"] }] }))).toEqual([]);
  });

  it("what can be done in each state", () => {
    const v = (status: SessionView["status"]) => ({ ...mockSessionView("working"), status });
    expect(canSend(v("idle"))).toBe(true);
    expect(canSend(v("needs_you"))).toBe(true);
    expect(canSend(v("stopped"))).toBe(false);
    expect(canSend(null)).toBe(false);
    expect(canInterrupt(v("working"))).toBe(true);
    expect(canInterrupt(v("idle"))).toBe(false);
    expect(pendingCards(mockSessionView("card")).map((c) => c.id)).toEqual([6]);
    expect(pendingCards(mockSessionView("idle"))).toEqual([]);
  });

  it("the sidebar indicator", () => {
    expect(indicator(mockSessionView("card"))).toEqual({ text: "Agent czeka na zgodę", attention: true });
    expect(indicator(mockSessionView("working"))?.text).toBe("Agent pracuje…");
    expect(indicator(mockSessionView("idle"))).toEqual({ text: "Propozycje gotowe (3)", attention: true });
    expect(indicator({ ...mockSessionView("idle"), status: "stopped" })?.attention).toBe(true);
    expect(indicator(null)).toBeNull();
  });

  it("tool names for people", () => {
    expect(toolLabel("mcp__slack__read_channel")).toBe("slack · read_channel");
    expect(toolLabel("mcp__montybrain__overview")).toBe("montybrain · overview");
    expect(toolLabel("computer-use/screenshot")).toBe("computer-use · screenshot");
    expect(toolLabel("Bash")).toBe("Bash");
  });
});

describe("assistant text in the panel", () => {
  it("shows the changes block as one line", async () => {
    const { displayText } = await import("./session");
    const t = "Raport.\n```montybrain-zmiany\n{\"changes\": []}\n```\nKoniec.";
    expect(displayText(t)).toBe("Raport.\n[blok zmian: propozycje do zaznaczenia]\nKoniec.");
    expect(displayText("```js\nx\n```")).toBe("```js\nx\n```");
  });
});

describe("context and complete question answers", () => {
  it("sends all questions together, including a free-text answer", async () => {
    const { completeAnswers } = await import("./session");
    const c = card({ questions: [
      { id: "channel", header: "", question: "Który kanał?", options: ["general", "projekt"] },
      { id: "range", header: "", question: "Jaki okres?", options: [] },
    ] });
    expect(completeAnswers(c, { channel: "general" })).toBeNull();
    expect(completeAnswers(c, { channel: "general", range: "  " })).toBeNull();
    expect(completeAnswers(c, { channel: "general", range: " ostatnie 3 dni ", unrelated: "omit" })).toEqual({ channel: "general", range: "ostatnie 3 dni" });
  });

  it("keeps topic focus separate from proposal scope and uses proposals rather than direct writes", async () => {
    const { sessionRequest } = await import("./session");
    const r = sessionRequest({ scope: "project", project: "firma", text: "Sprawdź odpowiedź", topic: { slug: "umowy", title: "Umowy" }, origin: "#/mapa" }, "codex");
    expect(r.scope).toBe("project");
    expect(r.project).toBe("firma");
    expect(r.text).toContain("[[firma/umowy]]");
    expect(r.text).toContain("propozycje do akceptacji");
    expect(r.text).not.toContain("log_update");
    const inbox = sessionRequest({ scope: "inbox", text: "Dopasuj", selection: ["a1"] }, "claude");
    expect(inbox.selection).toEqual(["a1"]);
    expect(inbox.project).toBeUndefined();
  });

  it("clears the proposal badge after decisions or replacement by another run", async () => {
    const { proposalPending } = await import("./session");
    const v = { ...mockSessionView("idle"), status: "stopped" as const };
    const update = { proposal: { run_id: v.id }, pending: 2 } as import("../api/types").ProjectUpdateView;
    expect(proposalPending(v, { "": update }, true)).toBe(2);
    expect(proposalPending(v, { "": { ...update, pending: 0 } }, true)).toBe(0);
    expect(proposalPending(v, { "": { ...update, proposal: { ...update.proposal!, run_id: "other" } } }, true)).toBe(0);
    expect(indicator(v, 0)?.attention).toBe(false);
  });
});

describe("consecutive tool groups", () => {
  it("keeps messages and permissions as boundaries, including pending cards", async () => {
    const { groupSessionEntries } = await import("./session");
    const source = mockSessionView("card").entries;
    const before = structuredClone(source);
    const groups = groupSessionEntries(source);
    expect(groups.map((e) => e.kind)).toEqual(["user", "notice", "assistant", "work", "permission", "permission"]);
    expect(groups[3]).toMatchObject({ id: 4, status: "running", latest: { id: 5 }, tools: [{ id: 4 }, { id: 5 }] });
    expect(source).toEqual(before);
    expect(groupSessionEntries([])).toEqual([]);
  });

  it("surfaces failures and keeps a running tool in the summary even when later tools finish", async () => {
    const { groupSessionEntries } = await import("./session");
    const tool = mockSessionView("working").entries.find((e) => e.kind === "tool")!;
    const entries = [{ ...tool, id: 1, status: "running" as const }, { ...tool, id: 2, status: "done" as const }, { ...tool, id: 3, status: "failed" as const }];
    expect(groupSessionEntries(entries)[0]).toMatchObject({ status: "failed", latest: { id: 1 }, tools: entries });
    expect(groupSessionEntries(entries.slice(1, 2))[0]).toMatchObject({ status: "done", latest: { id: 2 } });
    expect(groupSessionEntries([entries[0]!, card(), entries[1]!]).map((e) => e.kind)).toEqual(["work", "permission", "work"]);
  });
});

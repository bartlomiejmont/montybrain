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
    expect(indicator(mockSessionView("idle"))).toEqual({ text: "Agent: propozycje gotowe", attention: true });
    expect(indicator({ ...mockSessionView("idle"), status: "stopped" })).toBeNull();
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

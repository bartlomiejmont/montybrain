import { describe, expect, it } from "vitest";

import { createMockApi } from "./mock";
import { MOCK_COMMAND_STATES, MOCK_COMMAND_TEXT } from "./mock-command";

const now = () => new Date(2026, 10, 14, 10, 0);

describe("command field in the browser mock", () => {
  it("answers every URL state in the backend's shape", async () => {
    const api = createMockApi({ now, inbox: "demo" });
    for (const s of MOCK_COMMAND_STATES) {
      const { text, selection } = MOCK_COMMAND_TEXT[s];
      const r = await api.commandPreview({ text, selection, caret: text.length });
      expect(r.preview.text).toBe(text);
      expect(r.preview.reading.length, s).toBeGreaterThan(0);
    }
    const amb = await api.commandPreview({ text: MOCK_COMMAND_TEXT.ambiguous.text });
    expect(amb.preview.state).toBe("invalid");
    expect(amb.preview.plan).toBeNull();
    expect(amb.preview.choices.map((c) => c.choice.insert)).toEqual(["montybrain/onboarding", "whenubuy/onboarding"]);
    const typo = await api.commandPreview({ text: MOCK_COMMAND_TEXT.typo.text });
    expect(typo.preview.typo).toBe(true);
    expect(typo.preview.choices[0]!.choice.typo).toBe(true);
    const reading = await api.commandPreview({ text: MOCK_COMMAND_TEXT.reading.text, caret: MOCK_COMMAND_TEXT.reading.text.length });
    expect(reading.completions?.items[0]?.insert).toBe("whenubuy");
  });

  it("does not parse other text: it is a plain capture of that title", async () => {
    const api = createMockApi({ now });
    const r = await api.commandPreview({ text: "Kupić mleko !wysoki" });
    expect(r.preview.state).toBe("capture");
    expect(r.preview.plan!.steps[0]!.lines[0]).toMatch(/^- \[ \] Kupić mleko !wysoki \^[0-9a-z]{6}$/);
    expect((await api.commandPreview({ text: "? co z tym" })).preview.state).toBe("agent");
    expect((await api.commandPreview({ text: "  " })).preview.state).toBe("empty");
  });

  it("applies a previewed plan, refuses a changed one, and undoes", async () => {
    const api = createMockApi({ now, inbox: "demo" });
    const input = { text: MOCK_COMMAND_TEXT.topic.text, selection: MOCK_COMMAND_TEXT.topic.selection };
    const p = (await api.commandPreview(input)).preview;
    const stale = await api.commandApply(input, { ...p.plan!, steps: [] });
    expect(stale.kind).toBe("stale");
    const r = await api.commandApply(input, p.plan!);
    expect(r.kind).toBe("written");
    if (r.kind !== "written") return;
    expect(r.applied.echo[0]!.text).toBe("# Łazienka");
    let snap = api.snapshotForTests();
    expect(snap.tasks.find((t) => t.id === "d1e2m3")!.location).toEqual({ kind: "topic", project: "remont-kuchni", topic: "lazienka" });
    await api.commandUndo(r.undo, null);
    snap = api.snapshotForTests();
    expect(snap.tasks.find((t) => t.id === "d1e2m3")!.location).toEqual({ kind: "inbox" });
    expect(snap.topics.some((t) => t.slug === "lazienka")).toBe(false);
  });

  it("serves suggestions, a seeded inbox proposal and the agent's command", async () => {
    const api = createMockApi({ now, inboxProposal: true });
    const s = await api.inboxSuggestions();
    expect(s.find((x) => x.id === "d1e2m6")!.best!.place.text).toBe("whenubuy/platnosci-stripe-przelewy24");
    expect(s.find((x) => x.id === "k9x2m1")!.best).toBeNull();
    const u = await api.listUpdates();
    const kinds = u.find((x) => x.project === "")!.proposal!.changes.map((c) => c.change.kind);
    expect(kinds).toEqual(["topic_from_tasks", "move_task"]);
    expect((await api.inboxAgentAnswer("x")).command).toBe(MOCK_COMMAND_TEXT.agent.text);
  });
});

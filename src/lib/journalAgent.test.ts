import { describe, expect, it } from "vitest";
import type { JournalEntry } from "../api";
import { journalAgentBlock, newJournalEntries, journalRefreshMarker, journalRefreshLabel, journalRefreshRequest } from "./journalAgent";
import { mockProviders } from "../api/mock-runs";
import { buildFixtures } from "../api/fixtures";
import { buildJournalProposal } from "../api/mock-updates";
import { defaultSelection, describeChange, nextSelection, undecidedChanges } from "./updates";

describe("journal agent decisions", () => {
  it("explains unavailable, loading, running and undecided states", () => {
    expect(journalAgentBlock(false, null, 0, false)).toMatch(/Sprawdzam/);
    expect(journalAgentBlock(true, mockProviders("missing", "missing"), 0, false)).toMatch(/Żaden agent/);
    expect(journalAgentBlock(true, mockProviders("ok"), 2, false)).toMatch(/przejrzyj/);
    expect(journalAgentBlock(true, mockProviders("ok"), 0, true)).toMatch(/trwa już/);
    expect(journalAgentBlock(true, mockProviders("ok"), 0, false)).toBeNull();
  });

  it("keeps removals unchecked, actionable, visibly described and unchecked after undo", () => {
    const snap = buildFixtures("2026-11-15", "2026-11-15T15:00");
    const p = buildJournalProposal(snap, "whenubuy", "platnosci-stripe-przelewy24", "r", "codex", "2026-11-15T15:00", "Etap gotowy, usuń stare zadanie.");
    const removal = p.changes.findIndex((c) => c.change.kind === "remove_task");
    expect(removal).toBeGreaterThan(-1);
    expect(defaultSelection(p)).not.toContain(removal);
    expect(undecidedChanges(p)).toContain(removal);
    expect(describeChange(p.changes[removal]!.change).kind).toBe("Usunięcie zadania");
    expect([...nextSelection([], defaultSelection(p), new Set())]).not.toContain(removal);
  });
});

describe("journal refresh selection", () => {
  const entry = (text: string, line: number, at = "2026-11-15T15:00", author = "ja"): JournalEntry => ({ project: "p", topic: "t", at, author, text, line: 100 - line });
  it("starts with the last five user entries, ordered by timestamp then reverse line (Markdown is newest first)", () => {
    const entries = Array.from({ length: 7 }, (_, i) => entry(`${i}`, i));
    entries.push(entry("agent", 8, undefined, "codex"), entry("agent", 9, undefined, "claude-code"));
    expect(newJournalEntries(entries.reverse()).map((e) => e.text)).toEqual(["2", "3", "4", "5", "6"]);
    expect(newJournalEntries([])).toEqual([]);
  });
  it("distinguishes timestamps, content and duplicate occurrences, regardless of shifted lines", () => {
    const old = [entry("A", 10), entry("A", 11), entry("B", 12)];
    const marker = journalRefreshMarker(old)!;
    expect(newJournalEntries(old.map((e) => ({ ...e, line: e.line + 20 })), marker)).toEqual([]);
    expect(newJournalEntries([entry("A", 1), entry("A", 2), entry("A", 3), entry("B", 4), entry("C", 5)], marker).map((e) => e.text)).toEqual(["A", "C"]);
    expect(newJournalEntries([entry("A", 1), entry("edited B", 3)], marker).map((e) => e.text)).toEqual(["edited B"]);
    expect(newJournalEntries([entry("A", 1)], marker)).toEqual([]); // removal does not resurrect anything
    expect(newJournalEntries([entry("A", 1, "2026-11-16T15:00")], marker)).toHaveLength(1);
  });
  it("keeps entries added during a run new, including the same minute", () => {
    const old = [entry("A", 1)];
    const marker = journalRefreshMarker(old)!;
    expect(newJournalEntries([...old, entry("B", 2)], marker).map((e) => e.text)).toEqual(["B"]);
  });
  it.each([[0, "Brak nowych wpisów"], [1, "1 nowy wpis"], [2, "2 nowe wpisy"], [5, "5 nowych wpisów"], [12, "12 nowych wpisów"], [14, "14 nowych wpisów"], [21, "21 nowych wpisów"], [22, "22 nowe wpisy"], [112, "112 nowych wpisów"], [124, "124 nowe wpisy"]])("labels %s", (n, label) => {
    expect(journalRefreshLabel(n as number)).toBe(label);
  });
  it("quotes multiple saved entries with timestamps", () => {
    expect(journalRefreshRequest([entry("A", 1), entry("B", 2)])).toBe("Odświeżenie tematu · 2 nowe wpisy\n\n[2026-11-15T15:00] ja\nA\n\n[2026-11-15T15:00] ja\nB");
  });
});

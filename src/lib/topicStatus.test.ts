import { describe, expect, it } from "vitest";

import { EMPTY_SELECTION, openCount, bulkSummary, bulkTargets, clickSelect, pruneSelection, selectionLabel, statusChoices, statusPillLabel, type Selection } from "./topicStatus";

const order = ["a", "b", "c", "d", "e"];
const sel = (keys: string[], anchor: string | null): Selection => ({ selected: new Set(keys), anchor });
const keys = (s: Selection) => [...s.selected].sort();

describe("status pill and menu", () => {
  it("names the topic, its status and the action", () => {
    expect(statusPillLabel("Płatności BLIK", "in_progress")).toBe("Status tematu Płatności BLIK: W toku. Zmień");
  });

  it("lists the five statuses with the current one checked", () => {
    const c = statusChoices("waiting");
    expect(c.map((x) => x.label)).toEqual(["Planowany", "W toku", "Zablokowany", "Czeka", "Zrobiony"]);
    expect(c.filter((x) => x.checked).map((x) => x.status)).toEqual(["waiting"]);
    expect(statusChoices(null).some((x) => x.checked)).toBe(false);
  });
});

describe("bulk selection", () => {
  it("toggles single rows and remembers the anchor", () => {
    let s = clickSelect(order, EMPTY_SELECTION, "b", false);
    expect(keys(s)).toEqual(["b"]);
    expect(s.anchor).toBe("b");
    s = clickSelect(order, s, "d", false);
    expect(keys(s)).toEqual(["b", "d"]);
    s = clickSelect(order, s, "b", false);
    expect(keys(s)).toEqual(["d"]);
    expect(s.anchor).toBe("b");
  });

  it("selects a range with Shift, in both directions, keeping the anchor", () => {
    const s = clickSelect(order, sel(["b"], "b"), "d", true);
    expect(keys(s)).toEqual(["b", "c", "d"]);
    expect(s.anchor).toBe("b");
    // Shrinking the range does not unselect what is outside it.
    expect(keys(clickSelect(order, s, "a", true))).toEqual(["a", "b", "c", "d"]);
    expect(keys(clickSelect(order, sel(["e"], "e"), "c", true))).toEqual(["c", "d", "e"]);
  });

  it("clears a range with Shift when the anchor was unselected", () => {
    const s = clickSelect(order, sel(["a", "b", "c", "d"], "b"), "b", false); // b off, anchor b
    expect(keys(clickSelect(order, s, "d", true))).toEqual(["a"]);
  });

  it("treats Shift without a visible anchor as a plain click", () => {
    expect(keys(clickSelect(order, EMPTY_SELECTION, "c", true))).toEqual(["c"]);
    expect(keys(clickSelect(order, sel(["x"], "x"), "c", true))).toEqual(["c", "x"]);
    expect(clickSelect(order, EMPTY_SELECTION, "zz", false)).toBe(EMPTY_SELECTION);
  });

  it("drops rows that are no longer shown (done group closed)", () => {
    const s = sel(["a", "d", "e"], "e");
    const p = pruneSelection(s, ["a", "b", "c"]);
    expect(keys(p)).toEqual(["a"]);
    expect(p.anchor).toBeNull();
    expect(pruneSelection(p, ["a", "b", "c"])).toBe(p);
    expect(selectionLabel(3)).toBe("Zaznaczono 3");
  });

  it("changes only topics that do not have the status yet", () => {
    const topics = [
      { slug: "a", status: "planned" as const },
      { slug: "b", status: "in_progress" as const },
      { slug: "c", status: "planned" as const },
    ];
    expect(bulkTargets(topics, new Set(["a", "b"]), "in_progress").map((t) => t.slug)).toEqual(["a"]);
  });
});

describe("topic row counter", () => {
  it("uses Polish plurals", () => {
    expect(openCount(1, 1)).toBe("1 otwarte zadanie z 1");
    expect(openCount(2, 3)).toBe("2 otwarte zadania z 3");
    expect(openCount(5, 8)).toBe("5 otwartych zadań z 8");
    expect(openCount(0, 4)).toBe("0 otwartych zadań z 4");
    expect(openCount(22, 30)).toBe("22 otwarte zadania z 30");
  });
});

describe("bulk summary", () => {
  it("is one toast, with failures named", () => {
    const ok = (title: string) => ({ title, error: null });
    expect(bulkSummary([ok("A"), ok("B"), ok("C")], "in_progress")).toBe("Zmieniono status 3 tematów na „W toku”.");
    expect(bulkSummary([ok("A")], "done")).toBe("Zmieniono status 1 tematu na „Zrobiony”.");
    expect(bulkSummary([ok("A"), { title: "Kasa", error: "Nie ma takiego tematu." }, ok("C")], "waiting")).toBe(
      "Zmieniono status 2 z 3 tematów na „Czeka”. Nie udało się: „Kasa” (Nie ma takiego tematu.).",
    );
    expect(bulkSummary([{ title: "Kasa", error: "błąd zapisu" }], "waiting")).toBe("Nie udało się zmienić statusu: „Kasa” (błąd zapisu).");
    const many = ["A", "B", "C", "D", "E"].map((title) => ({ title, error: "x" }));
    expect(bulkSummary([ok("Z"), ...many], "blocked")).toBe(
      "Zmieniono status 1 z 6 tematów na „Zablokowany”. Nie udało się: „A” (x), „B” (x), „C” (x) i 2 więcej.",
    );
    expect(bulkSummary([], "planned")).toBe("Zaznaczone tematy mają już status „Planowany”.");
  });
});

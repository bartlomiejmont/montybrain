import { describe, expect, it } from "vitest";

import type { CommandPlan, CommandPreview, Completions } from "../api/types";
import { activeIndex, fieldList, initialField, readingShown, stepField, visibleItems, type FieldEffect, type FieldEvent, type FieldState } from "./fieldMachine";

const plan = (summary: string): CommandPlan => ({ confirm: true, steps: [{ op: { kind: "create_task", location: { kind: "inbox" }, task: { title: "x" } }, summary, file: "f.md", lines: ["- [ ] x ^······"] }] });

function reading(text: string, state: CommandPreview["state"], summary = "R"): CommandPreview {
  return {
    text,
    state,
    reading: summary,
    plan: state === "capture" || state === "confirm" ? { ...plan(summary), confirm: state === "confirm" } : null,
    problems: state === "invalid" || state === "typing" ? ["Problem"] : [],
    warnings: [],
    choices: [],
    typo: false,
    similar: [],
    agent: null,
  };
}

const comps = (from: number, to: number, inserts: string[]): Completions => ({ span: { from, to }, kind: "place", items: inserts.map((i) => ({ label: i, insert: i, detail: "" })) });

/** Runs events, collecting effects. */
function run(s: FieldState, ...events: FieldEvent[]): { s: FieldState; effects: FieldEffect[] } {
  const effects: FieldEffect[] = [];
  for (const e of events) {
    const r = stepField(s, e);
    s = r.state;
    effects.push(...r.effects);
  }
  return { s, effects };
}

const type = (s: FieldState, text: string) => stepField(s, { type: "edit", text, caret: text.length });

describe("command field decisions", () => {
  it("Enter with a destination fully typed shows the plan, not the suggestion", () => {
    let s = type(initialField(), "x > checkout").state;
    // The reading says one place; the first suggestion is another one.
    const r = run(
      s,
      { type: "reading", rev: s.rev, text: "x > checkout", preview: reading("x > checkout", "confirm", "Nowe zadanie w: Checkout"), completions: comps(4, 12, ["firma/checkout", "whenubuy/checkout"]) },
      { type: "enter" },
    );
    s = r.s;
    expect(visibleItems(s)).toHaveLength(0); // the list closes with the confirm step
    expect(s.text).toBe("x > checkout");
    expect(s.phase).toBe("confirm");
    expect(r.effects).toEqual([]);
    // The second Enter writes exactly the plan on screen.
    const w = run(s, { type: "enter" });
    expect(w.effects).toEqual([{ kind: "apply", plan: { ...plan("Nowe zadanie w: Checkout"), confirm: true }, text: "x > checkout" }]);
  });

  it("a suggestion equal to the word typed is not offered", () => {
    let s = type(initialField(), "x > whenubuy").state;
    s = run(s, { type: "reading", rev: s.rev, text: "x > whenubuy", preview: reading("x > whenubuy", "confirm"), completions: comps(4, 12, ["whenubuy", "whenubuy/checkout"]) }).s;
    expect(visibleItems(s).map((i) => i.insert)).toEqual(["whenubuy/checkout"]);
    expect(run(s, { type: "enter" }).s.phase).toBe("confirm");
  });

  it("Enter accepts a suggestion only after the arrows moved the highlight", () => {
    let s = type(initialField(), "x > wh").state;
    s = run(s, { type: "reading", rev: s.rev, text: "x > wh", preview: reading("x > wh", "invalid"), completions: comps(4, 6, ["whenubuy", "whenubuy/checkout"]) }).s;
    const r = run(s, { type: "arrow", dir: 1 }, { type: "arrow", dir: 1 }, { type: "enter" });
    expect(r.s.text).toBe("x > whenubuy/checkout");
    expect(r.effects.some((e) => e.kind === "fetch")).toBe(true);
    // Without the arrows, Enter reads the invalid text: a problem, no insertion.
    const r2 = run(s, { type: "enter" });
    expect(r2.s.text).toBe("x > wh");
    expect(r2.effects).toEqual([{ kind: "problem", message: "Problem" }]);
  });

  it("Tab accepts the first (or highlighted) suggestion", () => {
    let s = type(initialField(), "x > wh").state;
    s = run(s, { type: "reading", rev: s.rev, text: "x > wh", preview: reading("x > wh", "invalid"), completions: comps(4, 6, ["whenubuy", "whenubuy/checkout"]) }).s;
    expect(run(s, { type: "tab" }).s.text).toBe("x > whenubuy");
  });

  it("Enter within the debounce waits for the reading of the current text", () => {
    let s = type(initialField(), "Kupić").state;
    s = type(s, "Kupić mleko").state;
    const e = run(s, { type: "enter" });
    s = e.s;
    expect(s.waiting).toBe(true);
    expect(e.effects).toEqual([{ kind: "fetch", rev: s.rev, text: "Kupić mleko", caret: 11, now: true }]);
    // The answer for the older text arrives late: dropped.
    const late = run(s, { type: "reading", rev: s.rev - 1, text: "Kupić", preview: reading("Kupić", "capture"), completions: null });
    expect(late.effects).toEqual([]);
    expect(late.s.waiting).toBe(true);
    // The answer for this text: Enter acts now (plain capture writes).
    const now = run(late.s, { type: "reading", rev: s.rev, text: "Kupić mleko", preview: reading("Kupić mleko", "capture"), completions: null });
    expect(now.effects[0]).toMatchObject({ kind: "apply", text: "Kupić mleko" });
    expect(now.s.busy).toBe(true);
  });

  it("suggestions of older text are never applied to newer text", () => {
    let s = type(initialField(), "> mag").state;
    s = run(s, { type: "reading", rev: s.rev, text: "> mag", preview: reading("> mag", "invalid"), completions: comps(2, 5, ["magazyn"]) }).s;
    s = type(s, "> magazyn").state;
    // Tab and Enter on the new text with the old suggestions: no "magazynazyn".
    expect(visibleItems(s)).toHaveLength(0);
    expect(run(s, { type: "tab" }).s.text).toBe("> magazyn");
    const e = run(s, { type: "arrow", dir: 1 }, { type: "enter" });
    expect(e.s.text).toBe("> magazyn");
  });

  it("typing after Enter cancels the pending Enter; Esc steps back", () => {
    let s = type(initialField(), "a").state;
    s = run(s, { type: "enter" }).s;
    s = type(s, "ab").state;
    expect(s.waiting).toBe(false);
    s = run(s, { type: "reading", rev: s.rev, text: "ab", preview: reading("ab", "confirm"), completions: null }, { type: "enter" }).s;
    expect(s.phase).toBe("confirm");
    s = run(s, { type: "escape" }).s;
    expect(s.phase).toBe("edit");
    expect(run(s, { type: "escape" }).effects).toEqual([{ kind: "escape" }]);
  });

  it("a stale write shows the new plan and needs one more Enter", () => {
    let s = type(initialField(), "^a [x]").state;
    s = run(s, { type: "reading", rev: s.rev, text: "^a [x]", preview: reading("^a [x]", "confirm", "old"), completions: null }, { type: "enter" }, { type: "enter" }).s;
    expect(s.busy).toBe(true);
    s = run(s, { type: "stale", preview: reading("^a [x]", "confirm", "new") }).s;
    expect(s.phase).toBe("confirm");
    expect(s.busy).toBe(false);
    expect(run(s, { type: "enter" }).effects[0]).toMatchObject({ kind: "apply", plan: { steps: [{ summary: "new" }] } });
    // After a write the field is empty and asks for a fresh reading at once.
    const w = run(s, { type: "written" });
    expect(w.s.text).toBe("");
    expect(w.effects[0]).toMatchObject({ kind: "fetch", now: true, text: "" });
  });

  it("while the next reading is on its way the last one stays on screen, dimmed", () => {
    let s = type(initialField(), "x > wh").state;
    s = run(s, { type: "reading", rev: s.rev, text: "x > wh", preview: reading("x > wh", "typing", "Dokąd?"), completions: comps(4, 6, ["whenubuy", "whenubuy/checkout"]) }).s;
    s = type(s, "x > whe").state;
    // Nothing unmounts: the old reading and the old list are still shown…
    expect(readingShown(s)).toMatchObject({ stale: true, preview: { reading: "Dokąd?" } });
    expect(fieldList(s)).toMatchObject({ stale: true, source: "completions" });
    // …but keys do not act on them (and ↓ does not leave the field either).
    expect(visibleItems(s)).toHaveLength(0);
    expect(run(s, { type: "arrow", dir: 1 }).effects).toEqual([]);
    expect(run(s, { type: "tab" }).s.text).toBe("x > whe");
    // The new answer replaces them.
    s = run(s, { type: "reading", rev: s.rev, text: "x > whe", preview: reading("x > whe", "typing", "Dokąd?"), completions: comps(4, 7, ["whenubuy"]) }).s;
    expect(readingShown(s)?.stale).toBe(false);
    expect(fieldList(s)).toMatchObject({ stale: false });
    // An answer with no suggestions closes the list.
    s = type(s, "x > whenubuy ").state;
    s = run(s, { type: "reading", rev: s.rev, text: "x > whenubuy ", preview: reading("x > whenubuy ", "confirm"), completions: null }).s;
    expect(fieldList(s)).toBeNull();
  });
});

/** The keys and the mouse do the same on every list under the field. */
describe("every list under the field", () => {
  const choice = (from: number, to: number, insert: string, kind: string, typo = false) => ({ span: { from, to }, choice: { label: insert, insert, kind, typo } });

  interface Case {
    name: string;
    text: string;
    state: CommandPreview["state"];
    completions?: Completions;
    choices?: ReturnType<typeof choice>[];
    /** The text after taking the second row. */
    second: string;
    /** Esc after closing the list: back to editing (confirm) or out of the field. */
    phase?: "confirm";
  }

  const cases: Case[] = [
    { name: "destinations after ` > `", text: "x > wh", state: "typing", completions: comps(4, 6, ["whenubuy", "whenubuy/checkout"]), second: "x > whenubuy/checkout" },
    {
      name: "`/` command names",
      text: "^a1b2c3 /",
      state: "typing",
      completions: { span: { from: 8, to: 9 }, kind: "command", items: ["/temat ", "/czeka "].map((i) => ({ label: i.trim(), insert: i, detail: "", kind: "command" })) },
      second: "^a1b2c3 /czeka ",
    },
    {
      name: "tasks and topics by name after /czeka",
      text: "^a1b2c3 /czeka umo",
      state: "typing",
      completions: {
        span: { from: 15, to: 18 },
        kind: "ref",
        items: [
          { label: "Umowa najmu", insert: "^e5f6a7", detail: "Firma", kind: "task", marks: [{ from: 0, to: 3 }] },
          { label: "Umowy", insert: "[[firma/umowy]]", detail: "Firma · [[firma/umowy]]", kind: "topic", marks: [{ from: 0, to: 3 }] },
        ],
      },
      second: "^a1b2c3 /czeka [[firma/umowy]]",
    },
    {
      name: "the project for /temat",
      text: "/temat Łazienka",
      state: "typing",
      choices: [choice(15, 15, " > firma", "project"), choice(15, 15, " > remont", "project")],
      second: "/temat Łazienka > remont",
    },
    {
      name: "several places fit (ambiguity)",
      text: "x > checkout",
      state: "invalid",
      choices: [choice(4, 12, "firma/checkout", "topic"), choice(4, 12, "whenubuy/checkout", "topic")],
      second: "x > whenubuy/checkout",
    },
    {
      name: "a typo correction on the confirm step",
      text: "x > whenbuy",
      state: "confirm",
      choices: [choice(4, 11, "whenubuy", "project", true), choice(4, 11, "whenubuy/checkout", "topic", true)],
      second: "x > whenubuy/checkout",
      phase: "confirm",
    },
  ];

  function ready(c: Case): FieldState {
    let s = type(initialField(), c.text).state;
    const p = { ...reading(c.text, c.state), choices: c.choices ?? [] };
    s = run(s, { type: "reading", rev: s.rev, text: c.text, preview: p, completions: c.completions ?? null }).s;
    if (c.phase === "confirm") s = run(s, { type: "enter" }).s;
    return s;
  }

  for (const c of cases) {
    describe(c.name, () => {
      it("↓/↑ move the highlight and wrap around", () => {
        const s = ready(c);
        expect(fieldList(s)?.items).toHaveLength(2);
        const typing = c.state === "typing";
        // While typing the first row is highlighted from the start.
        expect(activeIndex(s)).toBe(typing ? 0 : null);
        const down = run(s, { type: "arrow", dir: 1 }).s;
        expect(activeIndex(down)).toBe(typing ? 1 : 0);
        expect(activeIndex(run(down, { type: "arrow", dir: 1 }, { type: "arrow", dir: 1 }).s)).toBe(typing ? 1 : 0);
        expect(activeIndex(run(s, { type: "arrow", dir: -1 }).s)).toBe(1);
      });

      it("Enter and Tab take the highlighted row", () => {
        const s = ready(c);
        const second = run(s, { type: "arrow", dir: -1 }).s; // ↑ from the top: the last row
        const e = run(second, { type: "enter" });
        expect(e.s.text).toBe(c.second);
        expect(e.s.phase).toBe("edit");
        expect(e.effects).toContainEqual({ kind: "caret", caret: c.second.length });
        expect(run(second, { type: "tab" }).s.text).toBe(c.second);
      });

      it("the mouse highlights what it hovers and takes what it clicks", () => {
        const s = ready(c);
        const hovered = run(s, { type: "hover", index: 1 }).s;
        expect(activeIndex(hovered)).toBe(1);
        // Keys continue from where the mouse is, and Enter takes that row.
        expect(activeIndex(run(hovered, { type: "arrow", dir: -1 }).s)).toBe(0);
        expect(run(hovered, { type: "enter" }).s.text).toBe(c.second);
        expect(run(s, { type: "pick", index: 1 }).s.text).toBe(c.second);
      });

      it("Esc closes the list first and cancels second", () => {
        const s = ready(c);
        const closed = run(s, { type: "escape" });
        expect(fieldList(closed.s)).toBeNull();
        expect(closed.effects).toEqual([]);
        expect(closed.s.text).toBe(c.text);
        const again = run(closed.s, { type: "escape" });
        if (c.phase === "confirm") expect(again.s.phase).toBe("edit");
        else expect(again.effects).toEqual([{ kind: "escape" }]);
        // Typing opens it again.
        expect(fieldList(type(closed.s, c.text + "a").state)).not.toBeNull();
      });
    });
  }

  it("typing: Enter with no list says what is missing instead of writing", () => {
    let s = type(initialField(), "^a1b2c3 /czeka ").state;
    s = run(s, { type: "reading", rev: s.rev, text: s.text, preview: reading(s.text, "typing", "Na co czeka?"), completions: null }).s;
    expect(run(s, { type: "enter" }).effects).toEqual([{ kind: "problem", message: "Problem" }]);
  });
});

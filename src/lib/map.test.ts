import { describe, expect, it } from "vitest";

import type { Task, Topic } from "../api/types";
import { buildTopicContext } from "./context";
import { COL_GAP, ISLAND_W, MARGIN, MAX_TASKS, ROW_GAP, TASK_H, TOPIC_H, layoutMap, overlaps, routeCurve, type LayoutInput, type LayoutRow } from "./mapLayout";
import { appendRefLine, buildGraph, counts, neighbours, parseRefs, referenceProse, removeRefLine, taskKey, topicKey, waitChain } from "./refs";

const topic = (project: string, slug: string, summary = "", created: string | null = "2026-11-01"): Topic => ({
  project,
  slug,
  title: slug,
  status: "in_progress",
  repo: null,
  summary,
  created,
  updated: null,
  path: `${project}/tematy/${slug}.md`,
});
const task = (id: string | null, project: string, t: string, notes = "", line = 1): Task => ({
  id,
  title: `Zadanie ${id}`,
  status: "todo",
  priority: "normal",
  due: null,
  done: null,
  links: [],
  notes,
  location: { kind: "topic", project, topic: t },
  line,
});

describe("references", () => {
  it("reads every form, the waits line, and skips code", () => {
    const text = [
      "Zależy od [[checkout]] i od [[montybrain/mapa]], zadanie ^abc123.",
      "Czeka na: [[whenubuy/platnosci]]",
      "Powiązane: [[ab-testy]]",
      "```",
      "[[w-kodzie]] ^zzz999",
      "```",
      "Kod `[[w-linii]]` się nie liczy, ani e-mail a^bc1234 ani [[Zły Slug]].",
    ].join("\n");
    const refs = parseRefs(text, "whenubuy");
    expect(refs.map((r) => [r.target, r.waits, r.standalone, r.line])).toEqual([
      [{ kind: "topic", project: "whenubuy", slug: "checkout" }, false, false, 0],
      [{ kind: "topic", project: "montybrain", slug: "mapa" }, false, false, 0],
      [{ kind: "task", id: "abc123" }, false, false, 0],
      [{ kind: "topic", project: "whenubuy", slug: "platnosci" }, true, true, 1],
      [{ kind: "topic", project: "whenubuy", slug: "ab-testy" }, false, true, 2],
    ]);
    // "Czeka na:" with prose around the reference is still "waits", but not removable as a line.
    expect(parseRefs("Czeka na: odpowiedź z [[a]] w sprawie KRS", "p")[0]).toMatchObject({ waits: true, standalone: false });
    expect(parseRefs("Czeka na: ^abc123", "p")[0]).toMatchObject({ target: { kind: "task", id: "abc123" }, waits: true, standalone: true });
  });

  it("builds the graph: same slug in two projects, dangling, missing ids, cycles", () => {
    const g = buildGraph({
      topics: [
        topic("a", "platnosci", "Czeka na: [[b/platnosci]]"),
        topic("b", "platnosci", "Czeka na: [[c]]\nPowiązane: [[a/platnosci]]"),
        topic("b", "c", "Czeka na: [[b/d]]"),
        topic("b", "d", "Czeka na: [[a/platnosci]]\n[[nie-ma]] i ^zz9999"),
      ],
      tasks: [task(null, "a", "platnosci", "Powiązane: [[b/c]]"), task("aaaaa1", "b", "c", "Czeka na: ^aaaaa1 (siebie)")],
    });
    const A = topicKey("a", "platnosci");
    const B = topicKey("b", "platnosci");
    expect(neighbours(g, A).map((n) => [n.other, n.edge.kind, n.outgoing])).toEqual([
      [B, "waits", true],
      [topicKey("b", "d"), "waits", false],
      [B, "related", false],
    ]);
    // A task without an id is a source with a fallback key, never a target.
    const fallback = taskKey(task(null, "a", "platnosci"));
    expect(fallback).toBe("k?:a/platnosci#1");
    expect(g.edges.some((e) => e.from === fallback && e.to === topicKey("b", "c"))).toBe(true);
    expect(g.edges.some((e) => e.to.startsWith("k?:"))).toBe(false);
    // Dangling references are warnings, not nodes; a self reference is dropped.
    expect(g.dangling.map((d) => d.text)).toEqual(["[[nie-ma]]", "^zz9999"]);
    expect(g.edges.some((e) => e.from === "k:aaaaa1" && e.to === "k:aaaaa1")).toBe(false);
    // The chain A → B → C → D closes back on A: cut there.
    expect(waitChain(g, A)).toEqual({ chain: [B, topicKey("b", "c"), topicKey("b", "d")], cycle: true });
    expect(waitChain(g, A, 2).chain).toHaveLength(2);
    expect(counts(g).get(A)).toEqual({ links: 3, waits: true });
  });

  it("appends and removes exactly one standalone line", () => {
    const target = topic("montybrain", "mapa");
    const before = "Integracja gotowa.\nBlokuje nas KRS.\n```\nCzeka na: [[montybrain/mapa]]\n```";
    const added = appendRefLine(before, "waits", target, "whenubuy");
    expect(added).toBe(`${before}\nCzeka na: [[montybrain/mapa]]`);
    // The one in the code block is not the line that makes the edge.
    expect(removeRefLine(added, "waits", { kind: "topic", project: "montybrain", slug: "mapa" }, "whenubuy")).toBe(before);
    expect(removeRefLine(before, "waits", { kind: "topic", project: "montybrain", slug: "mapa" }, "whenubuy")).toBeNull();
    expect(appendRefLine("", "related", target, "whenubuy")).toBe("Powiązane: [[montybrain/mapa]]");
    // In prose: not removable.
    expect(removeRefLine("Patrz [[montybrain/mapa]].", "related", { kind: "topic", project: "montybrain", slug: "mapa" }, "x")).toBeNull();
  });
});

function input(nProjects: number, nTopics: number, openPerTopic: number, expanded: Record<string, string> = {}): LayoutInput {
  const projects = Array.from({ length: nProjects }, (_, p) => ({
    slug: `p${p}`,
    topics: Array.from({ length: Math.ceil(nTopics / nProjects) }, (_, i) => ({ slug: `t${String(i).padStart(2, "0")}`, created: `2026-10-${String(1 + ((i * 7) % 28)).padStart(2, "0")}` })),
  }));
  return { projects, expanded, openTasks: (p, t) => Array.from({ length: openPerTopic }, (_, i) => `k:${p}${t}${i}`) };
}

const pos = (rows: LayoutRow[]) => Object.fromEntries(rows.map((r) => [r.key, r.rect]));

const noOverlap = (l: ReturnType<typeof layoutMap>) => {
  const rects = l.rows.map((r) => r.rect);
  for (let i = 0; i < rects.length; i++) for (let j = i + 1; j < rects.length; j++) expect(overlaps(rects[i]!, rects[j]!), `${l.rows[i]!.key} × ${l.rows[j]!.key}`).toBe(false);
  for (let i = 0; i < l.islands.length; i++) {
    for (let j = i + 1; j < l.islands.length; j++) expect(overlaps(l.islands[i]!.rect, l.islands[j]!.rect)).toBe(false);
    const isl = l.islands[i]!;
    for (const r of l.rows.filter((x) => x.project === isl.project)) expect(r.rect.y + r.rect.h).toBeLessThanOrEqual(isl.rect.y + isl.rect.h);
  }
};

describe("map layout (accordion)", () => {
  it("never overlaps at 40 topics and 150 open tasks, with any one topic expanded per island", () => {
    const inp = input(5, 40, 4);
    // About 150 open tasks: 12 on the first topic of each island, 3 on the others.
    inp.openTasks = (p, t) => Array.from({ length: t === "t00" ? 12 : 3 }, (_, i) => `k:${p}${t}${i}`);
    noOverlap(layoutMap(inp));
    // Every combination of the same expanded position in all islands, and mixed ones.
    for (let k = 0; k < 8; k++) {
      const t = `t${String(k).padStart(2, "0")}`;
      noOverlap(layoutMap({ ...inp, expanded: { p0: t, p1: t, p2: t, p3: t, p4: t } }));
      noOverlap(layoutMap({ ...inp, expanded: { p0: t, p2: `t0${(k + 3) % 8}`, p4: "t00" } }));
    }
    const all = layoutMap({ ...inp, expanded: { p0: "t00", p1: "t00", p2: "t00", p3: "t00", p4: "t00" } });
    expect(all.rows.filter((r) => r.kind === "task" && r.project === "p0")).toHaveLength(MAX_TASKS);
    expect(all.rows.find((r) => r.key === "more:p0/t00")).toBeTruthy();
  });

  it("packs islands with the normal gutter, no reserved space", () => {
    const l = layoutMap(input(4, 12, 0));
    // p0 and p2 in the left column: p2 starts one gutter below p0's frame.
    const [p0, , p2] = l.islands;
    expect(p2!.rect.y - (p0!.rect.y + p0!.rect.h)).toBe(ROW_GAP);
    expect(l.islands.map((i) => i.rect.x)).toEqual([MARGIN, MARGIN + ISLAND_W + COL_GAP, MARGIN, MARGIN + ISLAND_W + COL_GAP]);
  });

  it("orders topics by creation date, then slug, and nothing else", () => {
    const l = layoutMap({
      projects: [{ slug: "a", topics: [{ slug: "z", created: "2026-10-01" }, { slug: "b", created: "2026-11-01" }, { slug: "a", created: "2026-10-01" }, { slug: "x", created: null }] }],
      expanded: {},
      openTasks: () => [],
    });
    expect(l.rows.filter((r) => r.kind === "topic").map((r) => r.topic)).toEqual(["a", "z", "b", "x"]);
  });

  it("expanding pushes only what is below in the same column, by the block's height; collapsing restores", () => {
    const base = input(5, 30, 3);
    const a = layoutMap(base);
    const b = layoutMap({ ...base, expanded: { p0: "t02" } });
    const pa = pos(a.rows);
    const pb = pos(b.rows);
    const block = 3 * TASK_H + 4;
    const sorted = a.rows.filter((r) => r.project === "p0" && r.kind === "topic").map((r) => r.topic!);
    const below = new Set(sorted.slice(sorted.indexOf("t02") + 1));
    for (const r of a.rows) {
      const right = r.project === "p1" || r.project === "p3";
      const later = r.project === "p2" || r.project === "p4";
      const shifted = later || (r.project === "p0" && r.kind === "topic" && below.has(r.topic!));
      // The other column never moves; the clicked row and everything above it stay; the rest moves by the block.
      expect(pb[r.key], r.key).toEqual(right || !shifted ? pa[r.key] : { ...pa[r.key]!, y: pa[r.key]!.y + block });
    }
    expect(b.islands[2]!.rect.y - a.islands[2]!.rect.y).toBe(block);
    expect(b.islands[1]).toEqual(a.islands[1]);
    // Collapsing gives back exactly the first layout.
    expect(layoutMap({ ...base, expanded: { p0: undefined } })).toEqual(a);
  });

  it("data changes move nothing; a new topic appends and pushes its column by one row", () => {
    const base = input(4, 20, 2);
    const a = layoutMap({ ...base, expanded: { p1: "t01" } });
    // A task added to a collapsed topic, a journal entry, a status change: the layout does not read them.
    const more = (p: string, t: string) => Array.from({ length: t === "t01" && p === "p1" ? 2 : 7 }, (_, i) => `k:${p}${t}${i}`);
    expect(layoutMap({ ...base, expanded: { p1: "t01" }, openTasks: more })).toEqual(a);
    // A new (newest) topic in p0: it appends; p0's other rows stay; p2 (below, same column) moves one row; p1 and p3 do not.
    const added = { ...base, expanded: { p1: "t01" }, projects: base.projects.map((p, i) => (i === 0 ? { ...p, topics: [...p.topics, { slug: "nowy", created: "2026-12-01" }] } : p)) };
    const c = layoutMap(added);
    const pa = pos(a.rows);
    const pc = pos(c.rows);
    for (const r of a.rows) {
      const expected = r.project === "p2" ? { ...pa[r.key]!, y: pa[r.key]!.y + TOPIC_H } : pa[r.key];
      expect(pc[r.key], r.key).toEqual(expected);
    }
    const p0 = c.rows.filter((r) => r.project === "p0" && r.kind === "topic");
    expect(p0[p0.length - 1]!.topic).toBe("nowy");
  });
});

describe("Kopiuj kontekst with neighbours", () => {
  it("lists one-hop neighbours with their first sentence", () => {
    const text = buildTopicContext({
      project: { slug: "a", name: "A", color: null, state: "active", repos: [], description: "", path: "a/projekt.md" },
      topic: topic("a", "x", "Opis."),
      journal: [],
      tasks: [],
      branches: [],
      neighbours: [
        { kind: "Czeka na", title: "Płatności", project: "B", sentence: "Integracja gotowa." },
        { kind: "Powiązane", title: "Mapa", project: "A", sentence: "" },
      ],
    });
    expect(text).toContain("## Połączone tematy\n\n- Czeka na: B / Płatności — Integracja gotowa.\n- Powiązane: A / Mapa\n");
  });
});

describe("routeCurve", () => {
  const nums = (d: string) => [...d.matchAll(/-?\d+(?:\.\d+)?/g)].map((m) => Number(m[0]));

  it("anchors endpoints on island borders and routes right-column edges inside the middle gutter", () => {
    const l = layoutMap(input(4, 16, 1));
    const [p0, p1, , p3] = l.islands;
    const t0 = l.rows.find((r) => r.project === "p0" && r.kind === "topic")!;
    const t1 = l.rows.find((r) => r.project === "p1" && r.kind === "topic")!;
    const t3 = l.rows.find((r) => r.project === "p3" && r.kind === "topic")!;

    // Cross-column: left island right edge (508) -> right island left edge (628), never inside text or spine.
    const cross = routeCurve(t0.rect, t1.rect, p0!.rect, p1!.rect, 0);
    expect(cross).toMatch(/^M 508 /);
    expect(cross).toMatch(/ H 628$/);

    // Same right column: exits/enters on island left edge (628) through middle gutter, staying inside layout.width.
    const sameRight = routeCurve(t1.rect, t3.rect, p1!.rect, p3!.rect, 2);
    expect(sameRight).toMatch(/^M 628 /);
    expect(sameRight).toMatch(/ H 628$/);
    const xs: number[] = [];
    for (const [, cmd, raw] of sameRight.matchAll(/([MHQV])\s+([^MHQV]+)/g)) {
      const n = nums(raw!);
      if (cmd === "M" || cmd === "H") xs.push(n[0]!);
      else if (cmd === "Q") xs.push(n[0]!, n[2]!);
    }
    expect(xs.length).toBeGreaterThan(0);
    for (const x of xs) {
      expect(x).toBeGreaterThanOrEqual(p0!.rect.x + p0!.rect.w);
      expect(x).toBeLessThanOrEqual(l.width);
    }
  });
});


describe("relation prose in the map inspector", () => {
  it("keeps explanatory prose and code while removing only standalone relation lines", () => {
    const text = ["Stan wdrożenia.", "Czeka na: [[firma/krs]]", "Zależy też od [[api]] po zatwierdzeniu.", "Powiązane: ^abc123", "```", "Czeka na: [[przyklad]]", "```"].join("\n");
    expect(referenceProse(text, "sklep")).toBe(["Stan wdrożenia.", "Zależy też od [[api]] po zatwierdzeniu.", "```", "Czeka na: [[przyklad]]", "```"].join("\n"));
    expect(referenceProse("Czeka na: [[krs]]", "firma")).toBe("");
  });
});

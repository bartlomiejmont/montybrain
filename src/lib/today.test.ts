import { describe, expect, it } from "vitest";

import { buildFixtures } from "../api/fixtures";
import type { ProjectDrift, ProjectUpdateView, Snapshot, WaitView } from "../api/types";
import { buildToday, firstSentence, waitText, type Row } from "./today";

const TODAY = "2026-11-14";
const snap = (): Snapshot => buildFixtures(TODAY, `${TODAY}T12:00`);

const wait = (key: string, over: Partial<WaitView> = {}): WaitView => {
  const [, rest = ""] = key.split(":");
  const [project, topic] = rest.split("/");
  return {
    key,
    kind: key.startsWith("task:") ? "task" : "topic",
    project: project ?? null,
    topic: topic ?? null,
    task: key.startsWith("task:") ? rest : null,
    title: topic ?? rest,
    status: "waiting",
    reason: null,
    next: "2026-11-16",
    interval_days: 2,
    confirmed: null,
    since: "2026-11-12T10:00",
    due: false,
    ...over,
  };
};

const drift = (over: Partial<ProjectDrift> = {}): ProjectDrift => ({
  project: "whenubuy",
  collected_at: `${TODAY}T11:00`,
  branches: [],
  unknown: [],
  spread: [],
  moved: [],
  new_commits: null,
  since: null,
  links: [],
  ...over,
});

const keys = (rows: Row[]) => rows.map((r) => r.key);

describe("Dziś as an overview of topics", () => {
  it("puts each topic once, in the highest section that applies", () => {
    // The kitchen project is paused in the fixture: make it active here.
    const sn = snap();
    sn.projects = sn.projects.map((p) => (p.slug === "remont-kuchni" ? { ...p, state: "active" } : p));
    const s = buildToday({
      snapshot: sn,
      today: TODAY,
      waiting: [
        wait("topic:whenubuy/platnosci-stripe-przelewy24", { status: "blocked", next: TODAY, due: true }),
        wait("topic:whenubuy/powiadomienia-push", { next: "2026-11-15" }),
        wait("topic:remont-kuchni/wybor-blatu", { next: "2026-11-17", reason: "wycena konglomeratu" }),
      ],
      drift: [],
      updates: [],
    });
    const all = [...s.attention, ...s.active, ...s.waiting].map((r) => r.key);
    expect(new Set(all).size).toBe(all.length);
    // Overdue task + due check (Płatności), due-today tasks (A/B, Push).
    expect(keys(s.attention)).toEqual(["topic:whenubuy/platnosci-stripe-przelewy24", "topic:whenubuy/ab-testy-cen", "topic:whenubuy/powiadomienia-push"]);
    const p24 = s.attention[0]!;
    expect(p24.reasons.map((r) => r.kind)).toEqual(["check_due", "overdue"]);
    expect(p24.reasons[0]!.text).toBe("Zablokowany od 2 dni — czy coś się ruszyło?");
    expect(p24.reasons[1]!.text).toBe("1 zadanie po terminie");
    // In progress, most recent first; the waiting one with what and when.
    expect(keys(s.active)).toEqual(["topic:montybrain/aplikacja-desktopowa", "topic:montybrain/serwer-mcp"]);
    expect(s.active[0]!.reasons[0]!.text).toBe("Ostatni wpis dziś");
    expect(keys(s.waiting)).toEqual(["topic:remont-kuchni/wybor-blatu"]);
    expect(s.waiting[0]!.reasons[0]!.text).toBe("Czeka na: wycena konglomeratu · sprawdzimy 17 lis");
    // Planned and done topics are not listed.
    expect(all).not.toContain("topic:whenubuy/refaktoryzacja-koszyka");
    expect(all).not.toContain("topic:whenubuy/onboarding");
  });

  it("lists only urgent tasks outside topics, never a topic's task", () => {
    const s = buildToday({ snapshot: snap(), today: TODAY, waiting: [], drift: [], updates: [] });
    // Inbox: overdue a1b2c3; project tasks are not due; topic tasks never.
    expect(s.loose.map((t) => t.id)).toEqual(["a1b2c3"]);
    expect(s.loose.every((t) => t.location.kind !== "topic")).toBe(true);
  });

  it("raises stale topics, drift and proposals; a blocked loose task waits or asks", () => {
    const sn = snap();
    // Serwer MCP: last entry 9 days ago.
    sn.journal = sn.journal.map((j) => (j.topic === "serwer-mcp" ? { ...j, at: "2026-11-05T08:12" } : j));
    sn.tasks.push({ id: "zz11zz", title: "Klucz API od dostawcy", status: "blocked", priority: "normal", due: null, done: null, links: [], notes: "", location: { kind: "project", project: "whenubuy" }, line: 999 });
    const updates = [{ project: "whenubuy", pending: 4 } as ProjectUpdateView];
    const s = buildToday({
      snapshot: sn,
      today: TODAY,
      waiting: [wait("task:zz11zz", { kind: "task", status: "blocked", due: true })],
      drift: [
        drift({
          unknown: [{ repo: "app", branch: "feature/loyalty", last_commit: TODAY, worktree: null, topic: null, matched_by: null, active: true, ignored: false, seen: null }],
          spread: [{ topic: "ab-testy-cen", branches: 4, repos: 2 }],
          moved: [{ topic: "ab-testy-cen", repo: "app", branch: "feature/ab", last_commit: TODAY, last_entry: "2026-11-12" }],
          new_commits: 12,
        }),
      ],
      updates,
    });
    const project = s.attention.find((r) => r.kind === "project")!;
    expect(project.reasons.map((r) => r.text)).toEqual(["4 propozycje czekają", "1 gałąź bez tematu", "12 nowych commitów od sprawdzenia"]);
    expect(s.attention[0]).toBe(project);
    const mcp = s.attention.find((r) => r.key === "topic:montybrain/serwer-mcp")!;
    expect(mcp.reasons.map((r) => r.text)).toEqual(["Bez wpisu od 9 dni"]);
    const ab = s.attention.find((r) => r.key === "topic:whenubuy/ab-testy-cen")!;
    expect(ab.reasons.map((r) => r.kind)).toEqual(["due_today", "spread", "moved"]);
    expect(ab.reasons[1]!.text).toBe("4 gałęzie w 2 repozytoriach");
    const task = s.attention.find((r) => r.kind === "task")!;
    expect(task.key).toBe("task:zz11zz");
    // Not due: it waits, visibly separate.
    const later = buildToday({ snapshot: sn, today: TODAY, waiting: [wait("task:zz11zz", { kind: "task", status: "blocked" })], drift: [], updates: [] });
    expect(later.waiting.some((r) => r.key === "task:zz11zz")).toBe(true);
    expect(later.loose.some((t) => t.id === "zz11zz")).toBe(false);
  });

  it("paused and archived projects stay out", () => {
    const sn = snap();
    sn.projects = sn.projects.map((p) => (p.slug === "montybrain" ? { ...p, state: "archived" } : p));
    const s = buildToday({ snapshot: sn, today: TODAY, waiting: [], drift: [], updates: [] });
    expect([...s.attention, ...s.active].some((r) => r.kind === "topic" && r.project === "montybrain")).toBe(false);
    expect(s.waiting.some((r) => r.key.includes("remont-kuchni")), "paused in the fixture").toBe(false);
  });

  it("small wording helpers", () => {
    expect(firstSentence("Wariant B idzie na 20% ruchu. Czekamy na dane.")).toBe("Wariant B idzie na 20% ruchu.");
    expect(firstSentence("Bez kropki")).toBe("Bez kropki");
    expect(waitText(wait("topic:a/b", { next: TODAY }), TODAY)).toBe("Czeka · do sprawdzenia");
    expect(waitText(wait("topic:a/b", { next: "2026-11-15", status: "blocked", reason: "KRS" }), TODAY)).toBe("Blokuje: KRS · sprawdzimy jutro");
  });
});

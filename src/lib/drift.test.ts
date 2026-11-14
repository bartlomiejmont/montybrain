import { describe, expect, it } from "vitest";

import type { ProjectDrift, ProposedChange, Topic } from "../api/types";
import { buildTopicContext } from "./context";
import { driftLine, driftRequest, similarTopic, titleWords } from "./drift";

const d: ProjectDrift = {
  project: "whenubuy",
  collected_at: "2026-11-14T11:00",
  branches: [],
  unknown: [
    { repo: "mobile", branch: "feature/loyalty", last_commit: "2026-11-13", worktree: "/r/mobile-loyalty", topic: null, matched_by: null, active: true, ignored: false, seen: null },
    { repo: "backend", branch: "spike/graphql", last_commit: "2026-11-11", worktree: null, topic: null, matched_by: null, active: true, ignored: false, seen: null },
    { repo: "backend", branch: "x", last_commit: "2026-11-11", worktree: null, topic: null, matched_by: null, active: true, ignored: false, seen: null },
  ],
  spread: [{ topic: "platnosci", branches: 4, repos: 2 }],
  moved: [],
  new_commits: 12,
  since: "2026-11-13T18:00",
  links: [],
};
const titles = (s: string) => (s === "platnosci" ? "Płatności" : s);

describe("drift wording", () => {
  it("one line, or nothing", () => {
    expect(driftLine(d, titles)).toBe("Rozjazd: 3 gałęzie bez tematu · „Płatności”: 4 gałęzie w 2 repozytoriach · 12 nowych commitów od sprawdzenia");
    expect(driftLine({ ...d, unknown: [], spread: [], new_commits: 0 }, titles)).toBeNull();
    expect(driftLine(null, titles)).toBeNull();
    expect(driftLine({ ...d, unknown: [d.unknown[0]!], spread: [], new_commits: 1 }, titles)).toBe("Rozjazd: 1 gałąź bez tematu · 1 nowy commit od sprawdzenia");
  });

  it("prepares the request for the agent", () => {
    const r = driftRequest(d, titles);
    expect(r).toContain("- mobile: feature/loyalty (ostatni commit 2026-11-13, drzewo robocze /r/mobile-loyalty)");
    expect(r).toContain("Temat „Płatności” ma 4 gałęzie w 2 repozytoriach.");
    expect(r).toContain("topic_log z polem branches");
  });
});

const topic = (slug: string, title: string, status: Topic["status"] = "in_progress"): Topic => ({
  project: "whenubuy",
  slug,
  title,
  status,
  repo: null,
  summary: "",
  created: null,
  updated: null,
  path: `whenubuy/tematy/${slug}.md`,
});
const newTopic = (title: string, branches: { repo: string; branch: string }[] = []): ProposedChange => ({
  kind: "new_topic",
  title,
  slug: title.toLowerCase(),
  status: "in_progress",
  summary: "",
  repo: null,
  text: "",
  tasks: [],
  reason: "",
  branches,
});

describe("Podobny do", () => {
  const topics = [topic("platnosci-stripe-przelewy24", "Płatności: Stripe → Przelewy24"), topic("ab-testy-cen", "A/B testy cen"), topic("onboarding", "Onboarding nowych użytkowników", "done")];
  it("finds a topic with shared word stems, not a weak or tied one", () => {
    expect(similarTopic(newTopic("Płatność BLIK przez Przelewy24"), topics)?.slug).toBe("platnosci-stripe-przelewy24");
    expect(similarTopic(newTopic("Testy A/B na stronie produktu"), topics)?.slug).toBe("ab-testy-cen");
    expect(similarTopic(newTopic("Program lojalnościowy"), topics)).toBeNull();
    expect(similarTopic(newTopic("Onboarding v2"), topics), "done topics are not offered").toBeNull();
    expect(titleWords("Nowy temat dla API")).toEqual(["api"]);
  });
  it("or a branch already linked to a topic", () => {
    const links = [{ repo: "backend", branch: "feature/blik", topic: "ab-testy-cen" }];
    expect(similarTopic(newTopic("Coś zupełnie innego", [{ repo: "backend", branch: "feature/blik" }]), topics, links)?.slug).toBe("ab-testy-cen");
  });
});

describe("Kopiuj kontekst", () => {
  it("is a deterministic Markdown block", () => {
    const text = buildTopicContext({
      project: {
        slug: "whenubuy",
        name: "WhenUBuy",
        color: null,
        state: "active",
        repos: [{ name: "backend", path: "/r/backend" }],
        description: "",
        path: "whenubuy/projekt.md",
      },
      topic: { ...topic("platnosci", "Płatności"), status: "blocked", summary: "Czekamy na KRS.", repo: "backend" },
      journal: Array.from({ length: 7 }, (_, i) => ({ project: "whenubuy", topic: "platnosci", at: `2026-11-1${4 - (i % 5)}T10:0${i}`, author: "ja", text: `Wpis ${i}\ndruga linia`, line: i })),
      tasks: [
        { id: "aaaaa1", title: "Webhook", status: "doing", priority: "normal", due: "2026-11-15", done: null, links: [], notes: "", location: { kind: "topic", project: "whenubuy", topic: "platnosci" }, line: 1 },
        { id: "aaaaa2", title: "Stare", status: "done", priority: "normal", due: null, done: "2026-11-01", links: [], notes: "", location: { kind: "topic", project: "whenubuy", topic: "platnosci" }, line: 2 },
      ],
      branches: [
        { repo: "backend", branch: "feature/p24", last_commit: "2026-11-13", worktree: "/r/backend-p24", topic: "platnosci", matched_by: "link", active: true, ignored: false, seen: "2026-11-14T11:00" },
        { repo: "backend", branch: "feature/blik", last_commit: "2026-11-12", worktree: null, topic: "platnosci", matched_by: "words", active: true, ignored: false, seen: null },
      ],
    });
    expect(text).toBe(`# WhenUBuy / Płatności

- Projekt: WhenUBuy (\`whenubuy\`)
- Temat: Płatności (\`platnosci\`)
- Status: Zablokowany
- Główne repozytorium: backend

## Stan obecny

Czekamy na KRS.

## Ostatnie wpisy w dzienniku

- 2026-11-14 10:00 · ja: Wpis 0 druga linia
- 2026-11-13 10:01 · ja: Wpis 1 druga linia
- 2026-11-12 10:02 · ja: Wpis 2 druga linia
- 2026-11-11 10:03 · ja: Wpis 3 druga linia
- 2026-11-10 10:04 · ja: Wpis 4 druga linia

## Otwarte zadania

- ^aaaaa1 Webhook (w toku · termin 2026-11-15)

## Repozytoria

- backend: /r/backend

## Gałęzie tematu

- backend: feature/blik (ostatni commit 2026-11-12)
- backend: feature/p24 (ostatni commit 2026-11-13, drzewo robocze /r/backend-p24, widziana 2026-11-14 11:00)

---
Zapisuj postęp w Morwisku dla tego tematu (projekt \`whenubuy\`, temat \`platnosci\`): wpisy przez \`log_update\`, statusy zadań przez \`update_task\`, nowe zadania przez \`create_task\` (serwer MCP \`morwisko\`).
`);
  });
});

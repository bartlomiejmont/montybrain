import { describe, expect, it } from "vitest";

import type { ProjectUpdateView, ProviderInfo, RunInfo, StoredChange, StoredProposal, UpdateResult } from "../api/types";
import {
  applySummary,
  autoLabel,
  counts,
  defaultSelection,
  describeChange,
  formatNext,
  formatWhen,
  agentMeta,
  HISTORY_CONSENT,
  historyToggle,
  panelLabel,
  groupChanges,
  heldText,
  hourChoices,
  plural,
  projectProblem,
  receiptText,
  recheckQuestion,
  reviewHeading,
  reviewStatus,
  secondaryLabel,
  showStrip,
  statusLine,
  stripProposalBlock,
  stripState,
  undoSummary,
  updateRunFor,
  HISTORY_OFF_NOTE,
  historyLine,
  runningStatus,
  shorten,
  decidedLine,
  nextSelection,
  progressText,
  receiptLine,
  requestNote,
  rejectSummary,
  restoreSummary,
} from "./updates";

const NOW = new Date(2026, 10, 14, 15, 30);

const run = (over: Partial<RunInfo> = {}): RunInfo => ({
  id: "r1",
  recipe: "aktualizacja",
  recipe_title: "Aktualizacja projektu",
  provider: "codex",
  mode: "background",
  project: "beeking",
  model: null,
  effort: null,
  status: "running",
  started_at: "2026-11-14T15:29:00",
  finished_at: null,
  result: null,
  error: null,
  events: [],
  ...over,
});

const task = (outcome: StoredChange["outcome"] = null): StoredChange => ({
  change: { kind: "task_status", task: "aaaaa1", title: "Webhook", from: "doing", to: "done", reason: "Commit abc." },
  outcome,
});
const done = (status: "applied" | "stale" | "failed" | "rejected"): StoredChange["outcome"] => ({ status, message: "m", at: "2026-11-14T14:05" });

const proposal = (changes: StoredChange[], over: Partial<StoredProposal> = {}): StoredProposal => ({
  run_id: "r1",
  provider: "codex",
  trigger: "manual",
  created_at: "2026-11-14T14:02",
  report: "Raport.",
  changes,
  dropped: [],
  dropped_more: 0,
  problem: null,
  ...over,
});

const view = (over: Partial<ProjectUpdateView> = {}): ProjectUpdateView => ({
  project: "beeking",
  auto: { schedule: { kind: "off" }, provider: "claude", on_result: "propose" },
  sources: { history: false, history_days: 14 },
  next_due: null,
  last: null,
  proposal: null,
  pending: 0,
  held: false,
  undo: null,
  ...over,
});

const last = (outcome: UpdateResult["outcome"], over: Partial<UpdateResult> = {}): UpdateResult => ({
  at: "2026-11-14T14:02",
  provider: "codex",
  trigger: "manual",
  outcome,
  ...over,
});

const prov = (claude: boolean, codex: boolean): ProviderInfo[] => [
  { id: "claude", name: "Claude Code", available: claude, path: null, version: null, problem: null },
  { id: "codex", name: "Codex", available: codex, path: null, version: null, problem: null },
];

describe("update strip state", () => {
  it("running, review while undecided, receipt once everything is decided, else idle", () => {
    const r = run();
    expect(stripState(view({ proposal: proposal([task()]) }), r)).toEqual({ kind: "running", run: r });
    expect(stripState(view({ proposal: proposal([task(), task(done("applied"))]) }), null).kind).toBe("review");
    expect(stripState(view({ proposal: proposal([task(done("applied")), task(done("rejected"))]) }), null).kind).toBe("receipt");
    expect(stripState(view({ proposal: proposal([]) }), null).kind).toBe("idle");
    expect(stripState(view({ proposal: proposal([task()], { problem: "brak bloku" }) }), null).kind).toBe("idle");
    expect(stripState(null, null).kind).toBe("idle");
  });

  it("finds this project's running update run only", () => {
    const runs = [run({ id: "a", project: "dom" }), run({ id: "b", recipe: "postep" }), run({ id: "c", mode: "terminal" }), run({ id: "d", status: "succeeded" }), run({ id: "e" })];
    expect(updateRunFor(runs, "beeking")?.id).toBe("e");
    expect(updateRunFor(runs.slice(0, 4), "beeking")).toBeNull();
  });

  it("is shown only where there is something to compare", () => {
    expect(showStrip({ state: "active", repos: [{}] })).toBe(true);
    expect(showStrip({ state: "active", repos: [] })).toBe(false);
    expect(showStrip({ state: "archived", repos: [{}] })).toBe(false);
    const recipe = { needs_repos: true };
    expect(projectProblem(recipe, { name: "Dom", repos: [] })).toContain("Projekt „Dom” nie ma repozytoriów");
    expect(projectProblem(recipe, { name: "Dom", repos: [{}] })).toBeNull();
    expect(projectProblem({ needs_repos: false }, { name: "Dom", repos: [] })).toBeNull();
  });
});

describe("review labels never lie", () => {
  it("before anything is applied", () => {
    const c = counts(proposal([task(), task(), task(), task()]));
    expect(reviewHeading(c)).toBe("4 propozycje zmian");
    expect(reviewStatus(c)).toBe("Zaznacz, co zastosować.");
    expect(secondaryLabel(c)).toBe("Odrzuć wszystkie");
  });

  it("after a partial apply", () => {
    const one = counts(proposal([task(done("applied")), task(done("applied")), task(done("applied")), task()]));
    expect(reviewHeading(one)).toBe("3 z 4 rozstrzygnięte · 1 niezastosowana");
    expect(reviewStatus(one)).toBe("Zastosowano 3. Została 1 propozycja.");
    expect(secondaryLabel(one)).toBe("Odrzuć pozostałe (1)");
    const two = counts(proposal([task(done("applied")), task(), task()]));
    expect(reviewHeading(two)).toBe("1 z 3 rozstrzygnięte · 2 niezastosowane");
    expect(reviewStatus(two)).toBe("Zastosowano 1. Zostały 2 propozycje.");
    const five = counts(proposal([task(done("stale")), ...Array.from({ length: 5 }, () => task())]));
    expect(reviewHeading(five)).toBe("1 z 6 rozstrzygnięte · 5 niezastosowanych");
    expect(reviewStatus(five)).toBe("Zostało 5 propozycji.");
    expect(reviewHeading(five)).not.toContain("czeka");
  });

  it("nothing left: no secondary button, one receipt line", () => {
    const c = counts(proposal([task(done("applied")), task(done("applied")), task(done("applied")), task(done("stale"))]));
    expect(secondaryLabel(c)).toBeNull();
    expect(receiptText(c)).toBe("Zastosowano 3 zmiany · pominięto 1");
    expect(receiptText(counts(proposal([task(done("rejected")), task(done("rejected"))])))).toBe("Odrzucono 2 propozycje");
    expect(receiptText(counts(proposal([task(done("applied")), task(done("rejected")), task(done("failed"))])))).toBe(
      "Zastosowano 1 zmianę · 1 z błędem · odrzucono 1",
    );
  });

  it("held checks and the re-check question", () => {
    expect(heldText(1)).toBe("Czeka 1 propozycja. Automatyczne sprawdzenie wstrzymane.");
    expect(heldText(4)).toBe("Czekają 4 propozycje. Automatyczne sprawdzenie wstrzymane.");
    expect(heldText(5)).toBe("Czeka 5 propozycji. Automatyczne sprawdzenie wstrzymane.");
    expect(recheckQuestion(4)).toBe("Nowe sprawdzenie zastąpi 4 nieprzejrzane propozycje. Uruchomić?");
    expect(recheckQuestion(1)).toBe("Nowe sprawdzenie zastąpi 1 nieprzejrzaną propozycję. Uruchomić?");
    expect(recheckQuestion(7)).toContain("7 nieprzejrzanych propozycji");
    expect(recheckQuestion(2, true)).toBe("Zlecenie zastąpi 2 nieprzejrzane propozycje. Uruchomić?");
    expect(recheckQuestion(1, false, "claude")).toBe("Nowe sprawdzenie (Claude Code) zastąpi 1 nieprzejrzaną propozycję. Uruchomić?");
    expect(recheckQuestion(3, true, "codex")).toBe("Zlecenie (Codex) zastąpi 3 nieprzejrzane propozycje. Uruchomić?");
    expect(rejectSummary(5)).toBe("Odrzucono 5 propozycji.");
    expect(rejectSummary(1)).toBe("Odrzucono 1 propozycję.");
    const back = (status: "undone" | "changed") => ({ index: 0, status, message: "" });
    expect(restoreSummary([back("undone"), back("undone")])).toBe("Przywrócono do przeglądu 2 propozycje.");
    expect(restoreSummary([back("changed")])).toBe("Nic nie przywrócono: propozycje zmieniły się w międzyczasie.");
  });
});

describe("requests (zlecenie)", () => {
  it("are found as this project's running run and show their text, shortened", () => {
    const r = run({ id: "z", recipe: "zlecenie", request: "Znajdź na Slacku i w poczcie wszystko o płatnościach kartą i rozpisz temat na tej podstawie" });
    expect(updateRunFor([run({ id: "x", recipe: "postep" }), r], "beeking")?.id).toBe("z");
    expect(runningStatus(r)).toBe("Codex wykonuje zlecenie „Znajdź na Slacku i w poczcie wszystko o płatnościach kartą…”");
    expect(runningStatus(run())).toBe("Codex sprawdza projekt");
    expect(runningStatus({ ...r, request: "krótko", model: "opus", effort: null })).toBe("Codex wykonuje zlecenie „krótko” · opus");
    expect(shorten("a\n  b")).toBe("a b");
    expect(shorten("x".repeat(80), 10)).toBe("xxxxxxxxxx…");
  });

  it("have their own status wording, distinct from checks", () => {
    const req = { request: "Rozpisz temat kasy" };
    expect(statusLine(last({ kind: "proposed", count: 3 }, req), NOW).text).toBe("Zlecenie · dziś 14:02 · Codex · 3 propozycje");
    expect(statusLine(last({ kind: "no_changes" }, req), NOW).text).toBe("Zlecenie · dziś 14:02 · Codex · odpowiedź bez zmian w MontyBrain");
    expect(statusLine(last({ kind: "applied", count: 1 }, req), NOW).text).toBe("Zlecenie · dziś 14:02 · Codex · zastosowano 1 zmianę");
    expect(statusLine(last({ kind: "failed", message: "kod 3", limit: false }, req), NOW)).toEqual({
      text: "Zlecenie dziś 14:02 nie powiodło się · Codex",
      error: true,
      detail: "kod 3",
    });
    expect(statusLine(last({ kind: "cancelled" }, req), NOW).text).toBe("Przerwano zlecenie dziś 14:02 · Codex");
    // Limits are said the same way for both.
    const limit = last({ kind: "failed", message: "You have hit your usage limit.", limit: true }, req);
    expect(statusLine(limit, NOW, prov(true, true)).text).toBe("Limit Codex wykorzystany. Spróbuj później albo wybierz Claude Code.");
  });

  it("say when the history source is off", () => {
    expect(historyLine(HISTORY_OFF_NOTE, "review")).toBe("Historia rozmów jest wyłączona dla tego projektu (Źródła).");
    expect(historyLine(HISTORY_OFF_NOTE, "idle")).toBe(HISTORY_OFF_NOTE);
    expect(historyLine("Zebrano 2 sesje.", "review")).toBe("Źródła: repozytoria i historia rozmów. Zebrano 2 sesje.");
    expect(historyLine("Zebrano 2 sesje.", "idle")).toBe("Historia rozmów: Zebrano 2 sesje.");
    expect(historyLine(null, "idle")).toBeNull();
  });
});

describe("status line", () => {
  it("describes the last result, errors in words with a way out", () => {
    expect(statusLine(null, NOW).text).toBe("Porówna repozytoria z tematami i zadaniami i zaproponuje zmiany.");
    expect(statusLine(null, NOW, null, false).text).toBe("Znajdzie w repozytoriach tematy, nad którymi trwa praca, i zaproponuje je do dodania.");
    expect(statusLine(last({ kind: "no_changes" }), NOW).text).toBe("Sprawdzono dziś 14:02 · Codex · bez zmian");
    expect(statusLine(last({ kind: "proposed", count: 3 }), NOW).text).toBe("Sprawdzono dziś 14:02 · Codex · 3 propozycje");
    expect(statusLine(last({ kind: "applied", count: 3 }), NOW).text).toBe("Sprawdzono dziś 14:02 · Codex · zastosowano 3 zmiany");
    expect(statusLine(last({ kind: "unchanged" }, { trigger: "auto", at: "2026-11-13T09:00" }), NOW).text).toBe("Wczoraj 09:00 · bez zmian od ostatniego sprawdzenia");
    expect(statusLine(last({ kind: "failed", message: "kod 3", limit: false }), NOW)).toEqual({
      text: "Sprawdzanie dziś 14:02 nie powiodło się · Codex",
      error: true,
      detail: "kod 3",
    });
    expect(statusLine(last({ kind: "cancelled" }), NOW).text).toBe("Przerwano dziś 14:02 · Codex");
  });

  it("recognises limits and mentions the other provider only when it is available", () => {
    const limit = last({ kind: "failed", message: "You have hit your usage limit.\n(Codex CLI zakończył się kodem 1.)", limit: true });
    expect(statusLine(limit, NOW, prov(true, true))).toEqual({
      text: "Limit Codex wykorzystany. Spróbuj później albo wybierz Claude Code.",
      error: true,
      detail: "You have hit your usage limit.\n(Codex CLI zakończył się kodem 1.)",
    });
    expect(statusLine(limit, NOW, prov(false, true)).text).toBe("Limit Codex wykorzystany. Spróbuj później.");
    const claude = last({ kind: "failed", message: "Claude AI usage limit reached", limit: true }, { provider: "claude" });
    expect(statusLine(claude, NOW, prov(true, true)).text).toBe("Limit Claude Code wykorzystany. Spróbuj później albo wybierz Codex.");
  });
});

const newTopic = (title = "Program lojalnościowy", outcome: StoredChange["outcome"] = null): StoredChange => ({
  change: {
    kind: "new_topic",
    title,
    slug: "program-lojalnosciowy",
    status: "in_progress",
    summary: "Gałąź w toku (wniosek).",
    repo: "app",
    text: "feature/loyalty: 9 commitów.",
    tasks: [{ title: "Punkty", status: "doing" }],
    reason: "Aktywna gałąź.",
  },
  outcome,
});
const newTask = (): StoredChange => ({ change: { kind: "new_task", title: "Certyfikat", topic: null, topic_title: null, reason: "r" }, outcome: null });

describe("topics first", () => {
  it("groups topic changes before task changes", () => {
    const p = proposal([task(), newTopic(), newTask(), newTopic("Drugi")]);
    expect(groupChanges(p)).toEqual({ topics: [1, 3], tasks: [0, 2] });
    expect(groupChanges(proposal([task()]))).toEqual({ topics: [], tasks: [0] });
  });

  it("describes a new topic with its status, repo, summary, entry and tasks", () => {
    expect(describeChange(newTopic().change)).toEqual({
      kind: "Nowy temat",
      title: "Program lojalnościowy",
      what: "w toku · repozytorium app",
      detail: "feature/loyalty: 9 commitów.",
      summary: "Gałąź w toku (wniosek).",
      topic: true,
      detailLabel: "Pierwszy wpis w dzienniku",
      tasks: [{ title: "Punkty", status: "w toku" }],
    });
    expect(describeChange(newTask().change).what).toBe("poza tematami");
  });

  it("counts, heading and receipt account for new topics", () => {
    const fresh = counts(proposal([newTopic(), newTopic("B"), newTopic("C")]));
    expect(reviewHeading(fresh)).toBe("3 nowe tematy");
    expect(reviewHeading(counts(proposal([newTopic(), newTopic("B"), task(), newTask()])))).toBe("4 propozycje zmian, w tym 2 nowe tematy");
    expect(reviewHeading(counts(proposal([newTopic(), task()])))).toBe("2 propozycje zmian, w tym 1 nowy temat");
    const decided = counts(proposal([newTopic("A", done("applied")), newTopic("B", done("applied")), task(done("applied")), task(done("stale"))]));
    expect(receiptText(decided)).toBe("Dodano 2 tematy · zastosowano 1 zmianę · pominięto 1");
    expect(receiptText(counts(proposal(Array.from({ length: 5 }, (_, i) => newTopic(`T${i}`, done("applied"))))))).toBe("Dodano 5 tematów");
  });
});

describe("agents and history", () => {
  it("a rejected model is said plainly, the CLI text goes to details", () => {
    const l = statusLine(
      last({ kind: "failed", limit: false, model: true, message: "Codex nie przyjął modelu „gpt-x”. Zmień go w Ustawieniach.\n\nThe model `gpt-x` does not exist." }),
      NOW,
    );
    expect(l).toEqual({ text: "Codex nie przyjął modelu „gpt-x”. Zmień go w Ustawieniach.", error: true, detail: "The model `gpt-x` does not exist." });
  });

  it("shows the requested model and effort only when set", () => {
    expect(agentMeta({ model: null, effort: null })).toBeNull();
    expect(agentMeta({ model: "opus", effort: "high" })).toBe("opus · wysiłek high");
    expect(agentMeta({ model: null, effort: "xhigh" })).toBe("wysiłek xhigh");
    expect(panelLabel({ schedule: { kind: "every", hours: 6 }, provider: "claude", on_result: "propose" }, true)).toBe("Źródła: repozytoria i historia · auto co 6 h");
    expect(panelLabel({ schedule: { kind: "off" }, provider: "claude", on_result: "propose" }, false)).toBe("Źródła: repozytoria · auto wył.");
    expect(panelLabel({ schedule: { kind: "daily", hour: 9 }, provider: "codex", on_result: "apply" }, false)).toBe("Źródła: repozytoria · auto codziennie 09:00 · bez pytania");
  });

  it("turning the history on asks first; off saves at once", () => {
    const off = { history: false, history_days: 14 };
    expect(historyToggle(off, true)).toEqual({ ask: true, save: null });
    expect(historyToggle(off, true, true)).toEqual({ ask: false, save: { history: true, history_days: 14 } });
    expect(historyToggle({ history: true, history_days: 7 }, false)).toEqual({ ask: false, save: { history: false, history_days: 7 } });
    expect(historyToggle(off, false)).toEqual({ ask: false, save: null });
    expect(HISTORY_CONSENT).toContain("także do drugiego dostawcy");
  });
});

describe("formats and texts", () => {
  it("one time format everywhere", () => {
    expect(formatWhen("2026-11-14T09:05", NOW)).toBe("dziś 09:05");
    expect(formatWhen("2026-11-15T09:00", NOW)).toBe("jutro 09:00");
    expect(formatWhen("2026-11-02T18:00", NOW)).toBe("2 lis 18:00");
    expect(formatNext("2026-11-14T15:30", NOW)).toBe("wkrótce");
    expect(formatNext("2026-11-14T21:00", NOW)).toBe("dziś 21:00");
    expect(autoLabel({ schedule: { kind: "off" }, provider: "claude", on_result: "propose" })).toBe("wyłączona");
    expect(autoLabel({ schedule: { kind: "every", hours: 6 }, provider: "claude", on_result: "propose" })).toBe("co 6 h");
    expect(autoLabel({ schedule: { kind: "daily", hour: 9 }, provider: "codex", on_result: "apply" })).toBe("codziennie 09:00");
    expect([1, 2, 5, 12, 22, 112].map((n) => plural(n, "zmiana", "zmiany", "zmian"))).toEqual(["1 zmiana", "2 zmiany", "5 zmian", "12 zmian", "22 zmiany", "112 zmian"]);
    expect(hourChoices(5)).toEqual([1, 2, 3, 4, 5, 6, 8, 12, 24]);
  });

  it("says what each change does, nothing approved blind", () => {
    const t = describeChange(task().change);
    expect(t).toMatchObject({ kind: "Status zadania", title: "Webhook", what: "w toku → zrobione", detail: "Commit abc.", summary: null, topic: false });
    const log = describeChange({
      kind: "topic_log",
      topic: "p",
      topic_title: "Płatności",
      text: "Wpis",
      status: "waiting",
      summary: "Zwroty działają, czekamy na testy.",
      base_status: "in_progress",
      base_summary: "Stary",
    });
    expect(log).toMatchObject({
      topic: true,
      kind: "Dziennik i status tematu",
      title: "Płatności",
      what: "w toku → czeka",
      detail: "Wpis",
      summary: "Zwroty działają, czekamy na testy.",
    });
    const plain = describeChange({ kind: "topic_log", topic: "p", topic_title: "P", text: "W", status: null, summary: null, base_status: "planned", base_summary: "" });
    expect([plain.kind, plain.what, plain.summary]).toEqual(["Wpis w dzienniku", null, null]);
    expect(defaultSelection(proposal([task(), task(done("applied")), task()]))).toEqual([0, 2]);
  });

  it("sums up apply and undo", () => {
    expect(
      applySummary([
        { index: 0, status: "applied", message: "" },
        { index: 1, status: "applied", message: "" },
        { index: 2, status: "applied", message: "" },
      ]),
    ).toBe("Zastosowano 3 zmiany.");
    expect(applySummary([{ index: 0, status: "failed", message: "x" }])).toBe("Nic nie zastosowano; 1 z błędem.");
    expect(undoSummary([{ index: 0, status: "undone", message: "" }, { index: 1, status: "undone", message: "" }])).toBe("Cofnięto 2 zmiany.");
    expect(
      undoSummary([
        { index: 0, status: "undone", message: "" },
        { index: 1, status: "undone", message: "" },
        { index: 2, status: "changed", message: "" },
      ]),
    ).toBe("2 cofnięte, 1 zmienione w międzyczasie.");
  });

  it("hides the machine-readable block in Akcje", () => {
    const text = "Raport.\n\n- a\n\n```montybrain-zmiany\n{\"changes\": []}\n```\n";
    expect(stripProposalBlock(text)).toBe("Raport.\n\n- a");
    expect(stripProposalBlock("A\n```ts\nx\n```")).toBe("A\n```ts\nx\n```");
  });
});

describe("round 6", () => {
  it("keeps the user's ticks after a partial apply and ticks rows an undo brought back", () => {
    // Five rows; the user unticked 3 and 4 and applied 0–2.
    const afterApply = nextSelection([0, 1, 2, 3, 4], [3, 4], new Set([0, 1, 2]));
    expect([...afterApply]).toEqual([]);
    // The undo brings 0–2 back: they are ticked, 3 and 4 stay as the user left them.
    expect([...nextSelection([3, 4], [0, 1, 2, 3, 4], afterApply)].sort()).toEqual([0, 1, 2]);
    // A tick the user kept survives.
    expect([...nextSelection([0, 1, 2], [1, 2], new Set([2]))]).toEqual([2]);
  });

  it("collapses decided rows to one line and says who and when on the receipt", () => {
    expect(decidedLine(newTopic("Program lojalnościowy", done("applied")))).toBe("Nowy temat · Program lojalnościowy · Zastosowano");
    expect(decidedLine(task(done("stale")))).toBe("Status zadania · Webhook · Pominięto");
    const p = proposal([newTopic("A", done("applied")), task(done("applied")), task(done("applied")), task(done("applied")), task(done("rejected"))], { created_at: "2026-11-14T12:10" });
    expect(receiptLine(p, NOW)).toBe("Dziś 12:10 · Codex · dodano 1 temat · zastosowano 3 zmiany · odrzucono 1");
    expect(receiptLine({ ...p, request: "x" }, NOW)).toBe("Zlecenie · dziś 12:10 · Codex · dodano 1 temat · zastosowano 3 zmiany · odrzucono 1");
  });

  it("turns tool calls into Polish sentences", () => {
    const title = (s: string) => (s === "platnosci" ? "Płatności" : null);
    const tool = (text: string) => progressText({ kind: "tool", text }, title);
    expect(tool("overview {}")).toBe("Czytam przegląd projektu");
    expect(tool('get_topic {"project":"whenubuy","topic":"platnosci"}')).toBe("Czytam temat „Płatności”");
    expect(tool('get_topic {"project":"whenubuy","topic":"inny"}')).toBe("Czytam temat „inny”");
    expect(tool('list_tasks {"project":"whenubuy","status":"open"}')).toBe("Czytam listę zadań");
    expect(tool('search {"query":"blik"}')).toBe("Szukam „blik” w MontyBrain");
    expect(tool('search {"query":"blik i karty, a tu jest cięty tekst')).toBe("Szukam „blik i karty, a tu jest cięty tekst” w MontyBrain");
    expect(tool("mcp__montybrain__overview {}")).toBe("Czytam przegląd projektu");
    expect(tool('log_update {"topic":"x"}')).toBe("Agent próbował zapisać w MontyBrain (w tym trybie zablokowane)");
    expect(progressText({ kind: "note", text: "Zbieram stan repozytoriów (2; tylko odczyt)…" })).toBe("Zbieram stan repozytoriów (2; tylko odczyt)…");
    expect(progressText({ kind: "text", text: "I will now…" })).toBe("Agent analizuje i pisze odpowiedź…");
  });

  it("says in one line what the agent of a request can see", () => {
    expect(requestNote(true)).toBe("Agent widzi MontyBrain, repozytoria i historię rozmów; bez poczty i Slacka.");
    expect(requestNote(false)).toBe("Agent widzi MontyBrain i repozytoria; bez historii rozmów, poczty i Slacka.");
  });
});

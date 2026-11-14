// Mock of project updates („Aktualizacja"): proposals, applying, dismissing and
// the automatic-update setting, used in a plain browser and in tests. When a
// mock `aktualizacja` run ends, a proposal is built from the project's real
// (fixture) tasks; applying goes through the mock vault, so the views update.
// `scenario` seeds one state for screenshots (URL `?update=<scenario>`, see index.ts).

import type {
  ChangeEdit,
  DismissReport,
  JoinReport,
  ApplyItem,
  ApplyReport,
  AutoUpdate,
  UpdateSources,
  ChangeOutcome,
  ProjectUpdateView,
  ProviderId,
  RunInfo,
  Snapshot,
  StoredChange,
  StoredProposal,
  TaskLocation,
  Task,
  TaskStatus,
  TopicStatus,
  UndoReport,
  UndoResult,
  UpdateResult,
  JournalAdded,
  JournalEntry,
} from "./types";
import { toIsoMinute } from "../lib/dates";
import { HISTORY_OFF_NOTE } from "../lib/updates";
import { fold, slugify } from "../lib/text";

export type UpdateScenario = "never" | "running" | "proposals" | "topics" | "history" | "partial" | "applied" | "receipt" | "nochanges" | "error" | "unavailable" | "auto" | "pending" | "request" | "requestrun";
export const UPDATE_SCENARIOS: UpdateScenario[] = ["never", "running", "proposals", "topics", "history", "partial", "applied", "receipt", "nochanges", "error", "unavailable", "auto", "pending", "request", "requestrun"];

/** The project the scenarios are seeded on. */
export const SCENARIO_PROJECT = "whenubuy";
/** The empty project (repositories, no topics, no tasks) of the `topics` scenario. */
export const TOPICS_PROJECT = "beeking";

export const DEFAULT_AUTO: AutoUpdate = { schedule: { kind: "off" }, provider: "claude", on_result: "propose" };
export const DEFAULT_SOURCES: UpdateSources = { history: false, history_days: 14 };

/** What the mock history source "found" (synthetic). */
export const MOCK_HISTORY_NOTE = "Zebrano 12 sesji (Claude Code 9, Codex 3), 31 KB.";

/** What the mock history source "found" for a request (30 days, ranked by the request). */
export const MOCK_REQUEST_HISTORY_NOTE = "Zebrano 6 sesji (Claude Code 4, Codex 2), 14 KB.";
/** The request of the `request` / `requestrun` scenarios. */
export const MOCK_REQUEST = "Znajdź informacje o płatnościach BLIK na Slacku, w mailach i w T3 Code i rozpisz na tej podstawie temat";

/**
 * What a mock agent proposes for a request: writes up the matching (or first
 * open) topic of the project, or a new topic when the project has none. A
 * request ending with "?" is a question: answered in the report, no changes.
 */
export function buildRequestProposal(snap: Snapshot, project: string, runId: string, provider: ProviderId, at: string, request: string, history: boolean): StoredProposal {
  const words = fold(request).split(/[^a-z0-9]+/).filter((w) => w.length > 4);
  const topics = snap.topics.filter((t) => t.project === project && t.status !== "done");
  const topic = topics.find((t) => words.some((w) => fold(t.title).includes(w.slice(0, 5)))) ?? topics[0];
  const noAccess = /slack|mail|poczt/i.test(request) ? "Nie mam w tym trybie dostępu do poczty ani Slacka. " : "";
  const base = history ? "Opieram się na MontyBrain, repozytoriach i historii rozmów (sesje z 10.11 i 12.11)." : "Opieram się na MontyBrain i repozytoriach; historia rozmów jest wyłączona.";
  const proposal = (report: string, changes: StoredChange[]): StoredProposal => ({ run_id: runId, provider, trigger: "manual", created_at: at, report, changes, dropped: [], dropped_more: 0, problem: null, request, history_note: history ? MOCK_REQUEST_HISTORY_NOTE : HISTORY_OFF_NOTE });
  if (/\?\s*$/.test(request)) {
    return proposal(`${noAccess}${base}\n\nOstatnie commity w tym obszarze są z 12 lis (\`feature/blik-e2e\`), bez merge. Nic nie trzeba zmieniać w MontyBrain.`, []);
  }
  const report = `${noAccess}${base}\n\n- W historii rozmów: ustalono testy end-to-end na sandboksie i obsługę odrzuconych płatności (sesja Claude Code z 10.11).\n- W repozytorium: gałąź \`feature/blik-e2e\` z commitami z 12 i 13 lis, bez merge.\n\nRozpisuję temat na trzy kroki; maili i wątków ze Slacka nie sprawdziłem.`;
  if (!topic) {
    return proposal(report, [
      {
        change: {
          kind: "new_topic",
          title: "Płatności BLIK",
          slug: "platnosci-blik",
          status: "in_progress",
          summary: "Testy end-to-end na sandboksie w toku (gałąź feature/blik-e2e). Wniosek z rozmów: brakuje obsługi odrzuconych płatności.",
          repo: snap.projects.find((p) => p.slug === project)?.repos[0]?.name ?? null,
          text: "Na podstawie historii rozmów (10.11, 12.11) i gałęzi feature/blik-e2e.",
          tasks: [
            { title: "Testy end-to-end BLIK na sandboksie", status: "doing" },
            { title: "Obsługa odrzuconych płatności", status: "todo" },
          ],
          reason: "Zlecenie: rozpisać temat; w projekcie nie ma jeszcze tematów.",
        },
        outcome: null,
      },
    ]);
  }
  const task = (title: string, reason: string): StoredChange => ({ change: { kind: "new_task", title, topic: topic.slug, topic_title: topic.title, reason }, outcome: null });
  return proposal(report, [
    {
      change: {
        kind: "topic_log",
        topic: topic.slug,
        topic_title: topic.title,
        text: "Rozpisano temat na podstawie historii rozmów (10.11, 12.11) i gałęzi feature/blik-e2e: testy na sandboksie, odrzucone płatności, zwroty.",
        status: null,
        summary: null,
        base_status: topic.status,
        base_summary: topic.summary,
      },
      outcome: null,
    },
    task("Testy end-to-end BLIK na sandboksie", "Ustalone w sesji Claude Code z 10.11; gałąź feature/blik-e2e w toku."),
    task("Obsługa odrzuconych płatności BLIK", "Wniosek z sesji Codex z 12.11: brak ścieżki dla odmowy w aplikacji banku."),
    task("Zwroty dla płatności BLIK", "W rozmowach odłożone na później; brak śladu w repozytorium."),
  ]);
}

/** A planned topic that only the conversation history shows (the mock's history source). */
function historyTopic(project: string, repo: string | null): StoredChange {
  return {
    change: {
      kind: "new_topic",
      title: "Eksport faktur do KSeF",
      slug: "eksport-faktur-do-ksef",
      status: "planned",
      summary: "Postanowione w rozmowach z agentem, w repozytorium jeszcze nic nie ma. Cel: wysyłka faktur do KSeF z panelu sprzedaży.",
      repo,
      text: "Sesja Claude Code z 10.11: ustalono zakres i kolejność; sesja Codex z 12.11: wybór biblioteki do podpisu. Brak commitów.",
      tasks: [
        { title: "Wybrać bibliotekę do podpisu XAdES", status: "todo" },
        { title: "Szkic integracji z API KSeF", status: "todo" },
      ],
      reason: `Cel z historii rozmów (sesje z 10.11 i 12.11), bez śladu w repozytorium projektu ${project}.`,
    },
    outcome: null,
  };
}

/** The topic file's `zaktualizowano` before and as written (an undo puts `before` back while it is still `written`). */
type Updated = { topic: string; written: string; before: string } | null;

type UndoItem = (
  | { index: number; kind: "task_status"; task: string; from: TaskStatus; to: TaskStatus }
  | { index: number; kind: "new_task"; task: string; title: string; location: TaskLocation }
  | { index: number; kind: "new_topic"; topic: string; stamp: string }
  | { index: number; kind: "topic_log"; topic: string; entry: JournalEntry; status: [TopicStatus, TopicStatus] | null; summary: [string, string] | null }
  | { index: number; kind: "rejected" }
) & { updated?: Updated };

interface ProjectState {
  clock: string | null;
  last: UpdateResult | null;
  proposal: StoredProposal | null;
  undo?: { token: string; runId: string; items: UndoItem[] } | null;
}

let undoCounter = 1;

/** Usage / rate-limit errors (mirrors `is_limit_error` in src-tauri/src/updates/store.rs). */
export function isLimitError(message: string): boolean {
  return /usage limit|rate.limit|limit reached|limit will reset|hit your limit|quota|too many requests|429|out of credits|credit balance is too low/i.test(message);
}

/** What the mock needs from the mock vault. */
export interface MockVaultOps {
  snapshot(): Snapshot;
  setTaskStatus(id: string, status: TaskStatus): Promise<unknown>;
  addJournal(project: string, topic: string, author: string, text: string, status: TopicStatus | null, summary: string | null): Promise<JournalAdded>;
  createTask(location: TaskLocation, title: string): Promise<Task>;
  /** Core's `create_topic_with`: the topic, its first entry and tasks; returns the slug and a stamp of what was written. */
  createTopicWith(
    project: string,
    topic: { title: string; status: TopicStatus; summary: string; repo: string | null },
    author: string,
    text: string,
    tasks: { title: string; status: TaskStatus }[],
  ): Promise<{ slug: string; stamp: string }>;
  /** Core's `delete_topic_if`: removes the topic only while it is exactly as written (`stamp`). */
  deleteTopicIf(project: string, slug: string, stamp: string): Promise<boolean>;
  /** Stores a branch link (the drift sidecar); optional in tests. */
  link?(repo: string, branch: string, topic: string): void;
  /** Moves a task (Skrzynka proposals); optional in tests. */
  moveTask?(id: string, to: TaskLocation): Promise<unknown>;
  /** A topic's `zaktualizowano` (core's `stamp_of`) and putting an old value back (core's `restoring`). */
  updatedOf(project: string, topic: string): string | null;
  setUpdated(project: string, topic: string, value: string): void;
  /** Guarded undo primitives (core's `update_task_if`, `delete_task_if`, `revert_journal_entry`). */
  setTaskStatusIf(id: string, expected: TaskStatus, status: TaskStatus): Promise<boolean>;
  deleteTaskIf(id: string, guard: (t: Task) => boolean): Promise<boolean>;
  revertJournal(
    project: string,
    topic: string,
    entry: JournalEntry,
    status: [TopicStatus, TopicStatus] | null,
    summary: [string, string] | null,
  ): Promise<{ entry: boolean; status: boolean; summary: boolean }>;
}

const REPORT = `W projekcie „WhenUBuy” jedno zadanie wygląda na skończone, jedno trwa.

- **Obsłużyć webhook zwrotów** — commit \`a1c9f02\` „Obsługa refundów” na \`main\`, testy przechodzą.
- **Testy end-to-end płatności BLIK** — gałąź \`feature/blik-e2e\`, commity z ostatnich dwóch dni, bez merge.
- **Dashboard konwersji dla wariantu B** — brak nowych commitów od tygodnia; nie da się potwierdzić postępu.

Proponuję zamknąć webhook, dopisać stan płatności do dziennika i dodać zadanie na dokumentację zwrotów.`;

/** The report of a check that found nothing to change (the `nochanges` scenario). */
const NO_CHANGES_REPORT = `Tematy i zadania projektu „WhenUBuy” zgadzają się ze stanem repozytoriów; nic nie trzeba zmieniać.

- **Płatności: Stripe → Przelewy24** — status „zablokowany” nadal pasuje: od ostatniego wpisu brak nowych commitów na \`feature/p24\`.
- **A/B testy cen** — gałąź \`feature/ab-ceny\` ma commity z wczoraj; temat jest „w toku”, tak jak w MontyBrain.

Nie znalazłem nowych strumieni pracy ani zadań do zamknięcia.`;

const author = (p: ProviderId) => (p === "claude" ? "claude-code" : "codex");

const TOPICS_REPORT = `W repozytoriach widać trzy strumienie pracy z ostatnich dwóch tygodni; w MontyBrain nie ma jeszcze żadnego tematu.

- **Aplikacja kasowa (POS)** — gałąź \`feature/pos-offline\` w drzewie roboczym, 14 commitów od 3 lis, bez merge.
- **Integracja z hurtownią** — commity w \`api\` i \`app\` od 6 do 12 lis; wygląda na zakończoną (wniosek).
- **Migracja na Postgres 17** — gałąź zmergowana 2 lis, od tego czasu cisza.

Dwie gałęzie z jednym commitem (porządki) pominąłem: nie są osobnymi tematami.`;

/** Proposals for a project without topics: mostly new topics (what a mock agent finds in the repositories). */
export function buildTopicProposal(snap: Snapshot, project: string, runId: string, provider: ProviderId, trigger: "manual" | "auto", at: string): StoredProposal {
  const repos = snap.projects.find((p) => p.slug === project)?.repos.map((r) => r.name) ?? [];
  const main = repos[0] ?? null;
  const second = repos[1] ?? main;
  const changes: StoredChange[] = [
    {
      change: {
        kind: "new_topic",
        title: "Aplikacja kasowa w trybie offline",
        slug: "aplikacja-kasowa-w-trybie-offline",
        status: "in_progress",
        summary: "Trwa praca nad sprzedażą bez połączenia z siecią: kolejka transakcji i synchronizacja po powrocie sieci. Wniosek: celem jest praca na stoisku bez Wi-Fi.",
        repo: main,
        text: "Gałąź feature/pos-offline w drzewie roboczym (14 commitów od 3 lis, ostatni 13 lis: „Kolejka transakcji”).",
        tasks: [
          { title: "Synchronizacja kolejki po powrocie sieci", status: "doing" },
          { title: "Testy na tablecie bez sieci", status: "todo" },
        ],
        reason: "Aktywna gałąź w drzewie roboczym i świeże commity.",
      },
      outcome: null,
    },
    {
      change: {
        kind: "new_topic",
        title: "Integracja z hurtownią",
        slug: "integracja-z-hurtownia",
        status: "done",
        summary: `Import cennika i stanów z hurtowni działa (commity od 6 do 12 lis w ${main ?? "repozytorium"} i ${second ?? "repozytorium"}). Wniosek: zakończone, bo od 12 lis brak zmian.`,
        repo: second,
        text: "Commity 6–12 lis: „Import cennika”, „Mapowanie stanów”, „Harmonogram importu”; zmergowane do main.",
        tasks: [{ title: "Import cennika z hurtowni", status: "done" }],
        reason: "Seria powiązanych commitów w dwóch repozytoriach, zmergowana.",
      },
      outcome: null,
    },
    {
      change: {
        kind: "new_topic",
        title: "Migracja bazy na Postgres 17",
        slug: "migracja-bazy-na-postgres-17",
        status: "done",
        summary: "Migracja zmergowana 2 lis, od tego czasu bez zmian.",
        repo: main,
        text: "Gałąź chore/pg17 zmergowana 2 lis (7 commitów).",
        tasks: [],
        reason: "Duża, zakończona zmiana widoczna w historii.",
      },
      outcome: null,
    },
    {
      change: { kind: "new_task", title: "Odnowić certyfikat płatności", topic: null, topic_title: null, reason: "Certyfikat w konfiguracji wygasa 30 lis." },
      outcome: null,
    },
  ];
  return {
    run_id: runId,
    provider,
    trigger,
    created_at: at,
    report: TOPICS_REPORT,
    changes,
    dropped: [{ index: changes.length, label: "Nowe zadanie „Wypchnąć 2 commity”", reason: "Ponad limit 3 zadań poza tematami; zadania powinny należeć do tematów." }],
    dropped_more: 0,
    problem: null,
  };
}

/** A proposal built from the project's open tasks (what a mock agent "found"). */
export function buildProposal(snap: Snapshot, project: string, runId: string, provider: ProviderId, trigger: "manual" | "auto", at: string): StoredProposal {
  if (!snap.topics.some((t) => t.project === project)) return buildTopicProposal(snap, project, runId, provider, trigger, at);
  const inProject = (loc: TaskLocation) => loc.kind !== "inbox" && loc.project === project;
  const open = snap.tasks.filter((t) => t.id && t.status !== "done" && inProject(t.location));
  const changes: StoredChange[] = [];
  const doing = open.find((t) => t.title.toLowerCase().includes("webhook")) ?? open.find((t) => t.status === "doing") ?? open[0];
  if (doing?.id) {
    changes.push({
      change: { kind: "task_status", task: doing.id, title: doing.title, from: doing.status, to: "done", reason: "Commit a1c9f02 „Obsługa refundów” na main; testy webhooka przechodzą." },
      outcome: null,
    });
  }
  const second = open.find((t) => t !== doing && /blik/i.test(t.title)) ?? open.find((t) => t !== doing && t.status === "todo");
  if (second?.id) {
    changes.push({
      change: { kind: "task_status", task: second.id, title: second.title, from: second.status, to: "doing", reason: "Gałąź feature/blik-e2e ma commity z ostatnich dwóch dni, bez merge." },
      outcome: null,
    });
  }
  const topic = snap.topics.find((t) => t.project === project && t.status !== "done" && (doing?.location.kind === "topic" ? t.slug === doing.location.topic : true));
  if (topic) {
    changes.push({
      change: {
        kind: "topic_log",
        topic: topic.slug,
        topic_title: topic.title,
        text: "Webhook zwrotów zmergowany (a1c9f02). Testy BLIK w toku na gałęzi feature/blik-e2e.",
        status: topic.status === "waiting" ? null : "waiting",
        summary: topic.status === "waiting" ? null : "Webhook zwrotów działa na produkcji. Czekamy na testy BLIK na sandboksie i odpis z KRS dla Przelewy24.",
        base_status: topic.status,
        base_summary: topic.summary,
      },
      outcome: null,
    });
    changes.push({
      change: { kind: "new_task", title: "Opisać obsługę zwrotów w README", topic: topic.slug, topic_title: topic.title, reason: "Zwroty są w kodzie, ale nie ma ich w dokumentacji." },
      outcome: null,
    });
  }
  const repo = snap.projects.find((p) => p.slug === project)?.repos[0]?.name ?? null;
  if (!snap.topics.some((t) => t.project === project && fold(t.title) === fold("Program lojalnościowy"))) {
    changes.unshift({
      change: {
        kind: "new_topic",
        title: "Program lojalnościowy",
        slug: "program-lojalnosciowy",
        status: "in_progress",
        summary: "Punkty za zakupy i ekran nagród: gałąź feature/loyalty w drzewie roboczym, bez merge. Wniosek: pierwsza wersja dla klientów aplikacji mobilnej.",
        repo,
        text: "feature/loyalty: 9 commitów od 5 lis, ostatni 13 lis („Ekran nagród”); gałąź pobrana w drzewie roboczym.",
        tasks: [
          { title: "Naliczanie punktów po zakupie", status: "doing" },
          { title: "Ekran nagród", status: "todo" },
        ],
        reason: "Strumień pracy bez tematu w MontyBrain: aktywna gałąź i świeże commity.",
      },
      outcome: null,
    });
  }
  // A proposed topic that is really part of an existing one: the "Podobny do" hint.
  const push = snap.topics.find((t) => t.project === project && t.slug === "powiadomienia-push");
  if (push && push.status !== "done") {
    const at1 = changes.findIndex((c) => c.change.kind !== "new_topic");
    changes.splice(at1 < 0 ? changes.length : at1, 0, {
      change: {
        kind: "new_topic",
        title: "Powiadomienia push na Androidzie",
        slug: "powiadomienia-push-na-androidzie",
        status: "in_progress",
        summary: "Kanały powiadomień i FCM w aplikacji na Androida: gałąź push-notifications-android, bez merge.",
        repo: snap.projects.find((p) => p.slug === project)?.repos[1]?.name ?? repo,
        text: "push-notifications-android: 5 commitów od 10 lis, ostatni wczoraj („Kanały powiadomień”).",
        tasks: [
          { title: "Kanały powiadomień na Androidzie", status: "doing" },
          { title: "Odpowiedzieć na uwagi z App Store Review", status: "todo" },
        ],
        reason: "Aktywna gałąź bez tematu w MontyBrain.",
        branches: [{ repo: snap.projects.find((p) => p.slug === project)?.repos[1]?.name ?? repo ?? "whenubuy-mobile", branch: "push-notifications-android" }],
      },
      outcome: null,
    });
  }
  return {
    run_id: runId,
    provider,
    trigger,
    created_at: at,
    report: REPORT,
    changes,
    dropped: [{ index: changes.length, label: "Status zadania", reason: "Nie ma już takiego zadania w projekcie." }],
    dropped_more: 0,
    problem: null,
  };
}

function staleReason(c: StoredChange["change"], snap: Snapshot, project: string): string | null {
  if (c.kind === "move_task") {
    const t = snap.tasks.find((x) => x.id === c.task);
    return t && t.location.kind === "inbox" ? null : "zmienione w międzyczasie (wpis przeniesiono)";
  }
  if (c.kind === "topic_from_tasks") {
    const moved = c.tasks.some((e) => snap.tasks.find((x) => x.id === e.id)?.location.kind !== "inbox");
    return moved ? "zmienione w międzyczasie (wpis przeniesiono)" : null;
  }
  if (c.kind === "task_status") {
    const t = snap.tasks.find((x) => x.id === c.task);
    if (!t) return "zmienione w międzyczasie (zadania już nie ma)";
    if (t.status !== c.from) return "zmienione w międzyczasie (status zadania się zmienił)";
    return null;
  }
  if (c.kind === "topic_log") {
    const t = snap.topics.find((x) => x.project === project && x.slug === c.topic);
    if (!t) return "zmienione w międzyczasie (tematu już nie ma)";
    if (t.status !== c.base_status) return "zmienione w międzyczasie (status tematu się zmienił)";
    if (t.summary !== c.base_summary) return "zmienione w międzyczasie (opis tematu się zmienił)";
    return null;
  }
  if (c.kind === "new_topic") {
    const exists = snap.topics.some((t) => t.project === project && (t.slug === c.slug || fold(t.title) === fold(c.title)));
    return exists ? "zmienione w międzyczasie (taki temat już jest)" : null;
  }
  const loc: TaskLocation = c.topic ? { kind: "topic", project, topic: c.topic } : { kind: "project", project };
  const same = snap.tasks.some((t) => JSON.stringify(t.location) === JSON.stringify(loc) && fold(t.title) === fold(c.title));
  return same ? "zmienione w międzyczasie (takie zadanie już jest)" : null;
}

function nextDue(cfg: AutoUpdate, clock: string | null, now: Date): string | null {
  const s = cfg.schedule;
  if (s.kind === "off") return null;
  if (s.kind === "every") {
    if (!clock) return toIsoMinute(now);
    const d = new Date(clock);
    d.setHours(d.getHours() + s.hours);
    return toIsoMinute(d < now ? now : d);
  }
  const slot = new Date(now);
  slot.setHours(s.hour, 0, 0, 0);
  if (now >= slot) {
    if (!clock || new Date(clock) < slot) return toIsoMinute(now);
    slot.setDate(slot.getDate() + 1);
  }
  return toIsoMinute(slot);
}

export interface MockUpdatesOptions {
  now?: () => Date;
  scenario?: UpdateScenario;
}

export function createMockUpdates(
  vault: MockVaultOps,
  runs: {
    onRunUpdated(cb: (r: RunInfo) => void): Promise<() => void>;
    seedRunning?(req: { recipe: string; provider: ProviderId; mode: "background"; params: Record<string, string> }, elapsedSec: number, lines: { kind: "note" | "text" | "tool" | "error"; text: string }[]): RunInfo;
  },
  opts: MockUpdatesOptions = {},
) {
  const now = opts.now ?? (() => new Date());
  const states = new Map<string, ProjectState>();
  const auto = new Map<string, AutoUpdate>();
  const sources = new Map<string, UpdateSources>();
  const listeners = new Set<(u: ProjectUpdateView[]) => void>();
  const stamp = (minutesAgo = 0) => toIsoMinute(new Date(now().getTime() - minutesAgo * 60_000));
  const state = (p: string) => {
    let s = states.get(p);
    if (!s) states.set(p, (s = { clock: null, last: null, proposal: null }));
    return s;
  };

  function views(): ProjectUpdateView[] {
    const slugs = [...new Set([...states.keys(), ...auto.keys(), ...sources.keys()])].sort();
    return slugs.map((project) => {
      const s = states.get(project) ?? { clock: null, last: null, proposal: null };
      const cfg = auto.get(project) ?? DEFAULT_AUTO;
      return {
        project,
        auto: structuredClone(cfg),
        sources: structuredClone(sources.get(project) ?? DEFAULT_SOURCES),
        next_due: cfg.schedule.kind !== "off" && s.proposal?.changes.some((c) => !c.outcome) ? null : nextDue(cfg, s.clock, now()),
        last: structuredClone(s.last),
        proposal: structuredClone(s.proposal),
        pending: s.proposal?.changes.filter((c) => !c.outcome).length ?? 0,
        held: cfg.schedule.kind !== "off" && (s.proposal?.changes.some((c) => !c.outcome) ?? false),
        undo: s.undo && s.proposal && s.undo.runId === s.proposal.run_id ? s.undo.token : null,
        undo_rejection: !!(s.undo && s.proposal && s.undo.runId === s.proposal.run_id && s.undo.items.every((i) => i.kind === "rejected")),
      };
    });
  }
  const changed = () => {
    const v = views();
    setTimeout(() => listeners.forEach((cb) => cb(structuredClone(v))), 0);
  };

  // A finished mock `zlecenie` run: a proposal with its request; it is not a check (the clock stays).
  void runs.onRunUpdated((run) => {
    if (run.recipe !== "zlecenie" || run.mode !== "background" || !run.project || run.status === "running") return;
    const s = state(run.project);
    const at = stamp();
    const history = sources.get(run.project)?.history ?? false;
    const note = history ? MOCK_REQUEST_HISTORY_NOTE : HISTORY_OFF_NOTE;
    const request = run.request ?? "";
    if (run.status === "failed") s.last = { at, provider: run.provider, trigger: "manual", outcome: { kind: "failed", message: run.error ?? "Uruchomienie nie powiodło się.", limit: isLimitError(run.error ?? "") }, request, history_note: note };
    else if (run.status === "cancelled") s.last = { at, provider: run.provider, trigger: "manual", outcome: { kind: "cancelled" }, request, history_note: note };
    else {
      const p = buildRequestProposal(vault.snapshot(), run.project, run.id, run.provider, at, request, history);
      s.proposal = p;
      s.last = { at, provider: run.provider, trigger: "manual", outcome: p.changes.length ? { kind: "proposed", count: p.changes.length } : { kind: "no_changes" }, request, history_note: note };
    }
    changed();
  });

  // A finished mock `aktualizacja` run produces a proposal.
  void runs.onRunUpdated((run) => {
    if (run.recipe !== "aktualizacja" || run.mode !== "background" || !run.project || run.status === "running") return;
    const s = state(run.project);
    s.clock = run.started_at.slice(0, 16);
    const at = stamp();
    if (run.status === "failed") s.last = { at, provider: run.provider, trigger: "manual", outcome: { kind: "failed", message: run.error ?? "Uruchomienie nie powiodło się.", limit: isLimitError(run.error ?? "") } };
    else if (run.status === "cancelled") s.last = { at, provider: run.provider, trigger: "manual", outcome: { kind: "cancelled" } };
    else {
      const p = buildProposal(vault.snapshot(), run.project, run.id, run.provider, "manual", at);
      const src = sources.get(run.project);
      if (src?.history) {
        const repo = vault.snapshot().projects.find((x) => x.slug === run.project)?.repos[0]?.name ?? null;
        p.changes.unshift(historyTopic(run.project, repo));
        p.history_note = MOCK_HISTORY_NOTE;
      }
      s.proposal = p;
      s.last = {
        at,
        provider: run.provider,
        trigger: "manual",
        outcome: p.changes.length ? { kind: "proposed", count: p.changes.length } : { kind: "no_changes" },
        history_note: p.history_note ?? null,
      };
    }
    changed();
  });

  // --- scenarios (screenshots) ---
  const P = SCENARIO_PROJECT;
  const seedProposal = (trigger: "manual" | "auto", provider: ProviderId, minutesAgo: number) => {
    const s = state(P);
    const at = stamp(minutesAgo);
    s.clock = at;
    s.proposal = buildProposal(vault.snapshot(), P, `seed-${trigger}`, provider, trigger, at);
    s.last = { at, provider, trigger, outcome: { kind: "proposed", count: s.proposal.changes.length } };
    return s;
  };
  switch (opts.scenario) {
    case "request": {
      sources.set(P, { history: true, history_days: 14 });
      const s = state(P);
      s.last = { at: stamp(60 * 26), provider: "codex", trigger: "manual", outcome: { kind: "no_changes" } };
      const at = stamp(2);
      s.proposal = buildRequestProposal(vault.snapshot(), P, "seed-request", "claude", at, MOCK_REQUEST, true);
      s.last = { at, provider: "claude", trigger: "manual", outcome: { kind: "proposed", count: s.proposal.changes.length }, request: MOCK_REQUEST, history_note: MOCK_REQUEST_HISTORY_NOTE };
      break;
    }
    case "requestrun":
      sources.set(P, { history: true, history_days: 14 });
      state(P).last = { at: stamp(60 * 26), provider: "codex", trigger: "manual", outcome: { kind: "no_changes" } };
      runs.seedRunning?.({ recipe: "zlecenie", provider: "claude", mode: "background", params: { project: P, text: MOCK_REQUEST } }, 17, [
        { kind: "note", text: "Zbieram stan repozytoriów (2; tylko odczyt)…" },
        { kind: "note", text: "Zbieram historię rozmów (lokalnie, z redakcją)…" },
        { kind: "note", text: `Historia rozmów: ${MOCK_REQUEST_HISTORY_NOTE}` },
        { kind: "tool", text: 'get_topic {"project":"whenubuy","topic":"platnosci-stripe-przelewy24"}' },
      ]);
      break;
    case "running":
      state(P).last = { at: stamp(60 * 26), provider: "codex", trigger: "manual", outcome: { kind: "no_changes" } };
      runs.seedRunning?.({ recipe: "aktualizacja", provider: "codex", mode: "background", params: { project: P } }, 42, [
        { kind: "note", text: "Zbieram stan repozytoriów (2; tylko odczyt)…" },
        { kind: "note", text: "Zebrano stan 2 z 2 repozytoriów (5 KB)." },
        { kind: "note", text: "Uruchamiam Codex CLI w tle (MontyBrain tylko do odczytu)" },
        { kind: "tool", text: 'list_tasks {"project":"whenubuy"}' },
        { kind: "text", text: "Porównuję otwarte zadania z commitami z ostatnich 14 dni." },
      ]);
      break;
    case "history": {
      sources.set(P, { history: true, history_days: 14 });
      const s = seedProposal("manual", "codex", 6);
      const repo = vault.snapshot().projects.find((x) => x.slug === P)?.repos[0]?.name ?? null;
      s.proposal!.changes.unshift(historyTopic(P, repo));
      s.proposal!.history_note = MOCK_HISTORY_NOTE;
      s.proposal!.report = `${MOCK_HISTORY_NOTE.replace(/\.$/, "")}; dwie z nich dotyczą faktur, których nie ma jeszcze w repozytorium.\n\n` + s.proposal!.report;
      s.last = { ...s.last!, outcome: { kind: "proposed", count: s.proposal!.changes.length }, history_note: MOCK_HISTORY_NOTE };
      break;
    }
    case "topics": {
      const s = state(TOPICS_PROJECT);
      const at = stamp(3);
      s.clock = at;
      s.proposal = buildTopicProposal(vault.snapshot(), TOPICS_PROJECT, "seed-topics", "codex", "manual", at);
      s.last = { at, provider: "codex", trigger: "manual", outcome: { kind: "proposed", count: s.proposal.changes.length } };
      break;
    }
    case "proposals":
      seedProposal("manual", "codex", 4);
      break;
    case "applied": {
      const s = seedProposal("manual", "codex", 12);
      const p = s.proposal!;
      const ok = (message: string): ChangeOutcome => ({ status: "applied", message, at: stamp(1) });
      p.changes.forEach((c, i) => {
        c.outcome = i === 2 ? { status: "stale", message: "zmienione w międzyczasie (status zadania się zmienił)", at: stamp(1) } : ok("Zastosowano.");
      });
      s.last = { at: stamp(1), provider: "codex", trigger: "manual", outcome: { kind: "applied", count: p.changes.filter((c) => c.outcome?.status === "applied").length } };
      break;
    }
    case "nochanges": {
      const s = state(P);
      const at = stamp(18);
      s.clock = at;
      s.proposal = { ...buildProposal(vault.snapshot(), P, "seed-none", "codex", "manual", at), changes: [], dropped: [], report: NO_CHANGES_REPORT };
      s.last = { at, provider: "codex", trigger: "manual", outcome: { kind: "no_changes" } };
      break;
    }
    case "error":
      state(P).last = {
        at: stamp(7),
        provider: "codex",
        trigger: "manual",
        outcome: { kind: "failed", message: "You have hit your usage limit. Try again later.\n(Codex CLI zakończył się kodem 1.)", limit: true },
      };
      break;
    case "auto":
      auto.set(P, { schedule: { kind: "every", hours: 6 }, provider: "claude", on_result: "propose" });
      state(P).last = { at: stamp(95), provider: "claude", trigger: "auto", outcome: { kind: "unchanged" } };
      state(P).clock = stamp(95);
      break;
    case "partial": {
      const s = seedProposal("manual", "codex", 9);
      const p = s.proposal!;
      // Everything but one task change applied.
      const left = p.changes.findIndex((c) => c.change.kind === "task_status");
      p.changes.forEach((c, i) => {
        if (i !== left) c.outcome = { status: "applied", message: "Zastosowano.", at: stamp(1) };
      });
      s.last = { at: stamp(1), provider: "codex", trigger: "manual", outcome: { kind: "applied", count: p.changes.length - 1 } };
      break;
    }
    case "receipt": {
      const s = seedProposal("manual", "codex", 14);
      const p = s.proposal!;
      p.changes.forEach((c, i) => {
        c.outcome =
          i === 2
            ? { status: "stale", message: "zmienione w międzyczasie (status zadania się zmienił)", at: stamp(2) }
            : { status: "applied", message: "Zastosowano.", at: stamp(2) };
      });
      s.last = { at: stamp(2), provider: "codex", trigger: "manual", outcome: { kind: "applied", count: p.changes.length - 1 } };
      break;
    }
    case "pending":
      auto.set(P, { schedule: { kind: "every", hours: 12 }, provider: "codex", on_result: "propose" });
      seedProposal("auto", "codex", 50);
      break;
    default:
      break;
  }

  return {
    async listUpdates(): Promise<ProjectUpdateView[]> {
      return views();
    },
    /** Puts a proposal under `project` (`""` = Skrzynka) — mock URL states. */
    seedProposal(project: string, proposal: StoredProposal) {
      const s = state(project);
      s.proposal = structuredClone(proposal);
      s.last = { at: proposal.created_at, provider: proposal.provider, trigger: "manual", outcome: { kind: "proposed", count: proposal.changes.length }, request: proposal.request ?? null };
      changed();
    },
    async applyUpdate(runId: string, indices: number[]): Promise<ApplyReport> {
      const entry = [...states.entries()].find(([, s]) => s.proposal?.run_id === runId);
      if (!entry) throw new Error("Tej propozycji już nie ma: została odrzucona albo zastąpiona nowszą.");
      const [project, s] = entry;
      const p = s.proposal!;
      const items: ApplyItem[] = [];
      const written: UndoItem[] = [];
      for (const index of [...new Set(indices)]) {
        const sc = p.changes[index];
        if (!sc) {
          items.push({ index, status: "missing", message: `Nie ma zmiany nr ${index + 1}.` });
          continue;
        }
        if (sc.outcome?.status === "applied") {
          items.push({ index, status: "already", message: "Już zastosowane." });
          continue;
        }
        const stale = staleReason(sc.change, vault.snapshot(), project);
        let outcome: ChangeOutcome;
        if (stale) outcome = { status: "stale", message: stale, at: stamp() };
        else {
          const c = sc.change;
          // The topic file this write touches (its date is put back by a full undo).
          const touched =
            c.kind === "topic_log" ? c.topic : c.kind === "new_task" ? c.topic : c.kind === "task_status" ? (vault.snapshot().tasks.find((t) => t.id === c.task)?.location as { topic?: string } | undefined)?.topic ?? null : null;
          const before = touched ? vault.updatedOf(project, touched) : null;
          const n = written.length;
          try {
            if (c.kind === "task_status") {
              await vault.setTaskStatus(c.task, c.to);
              written.push({ index, kind: "task_status", task: c.task, from: c.from, to: c.to });
            } else if (c.kind === "topic_log") {
              const j = await vault.addJournal(project, c.topic, author(p.provider), c.text, c.status, c.summary);
              for (const b of c.branches ?? []) vault.link?.(b.repo, b.branch, c.topic);
              written.push({
                index,
                kind: "topic_log",
                topic: c.topic,
                entry: j.entry,
                status: c.status ? [j.topic.status, c.base_status] : null,
                summary: c.summary !== null ? [j.topic.summary, c.base_summary] : null,
              });
            } else if (c.kind === "move_task") {
              await vault.moveTask?.(c.task, c.to);
            } else if (c.kind === "topic_from_tasks") {
              const r = await vault.createTopicWith(c.project, { title: c.title, status: "planned", summary: c.summary, repo: null }, author(p.provider), "Temat z wpisów ze Skrzynki.", []);
              for (const e of c.tasks) await vault.moveTask?.(e.id, { kind: "topic", project: c.project, topic: r.slug });
            } else if (c.kind === "new_topic") {
              const r = await vault.createTopicWith(project, { title: c.title, status: c.status, summary: c.summary, repo: c.repo }, author(p.provider), c.text, c.tasks);
              written.push({ index, kind: "new_topic", topic: r.slug, stamp: r.stamp });
              for (const b of c.branches ?? []) vault.link?.(b.repo, b.branch, r.slug);
            } else {
              const location: TaskLocation = c.topic ? { kind: "topic", project, topic: c.topic } : project ? { kind: "project", project } : { kind: "inbox" };
              const t = await vault.createTask(location, c.title);
              written.push({ index, kind: "new_task", task: t.id!, title: t.title, location });
            }
            const after = touched ? vault.updatedOf(project, touched) : null;
            if (written.length > n && touched && before && after && before !== after) written[n]!.updated = { topic: touched, written: after, before };
            outcome = { status: "applied", message: "Zastosowano.", at: stamp() };
          } catch (e) {
            outcome = { status: "failed", message: e instanceof Error ? e.message : String(e), at: stamp() };
          }
        }
        sc.outcome = outcome;
        items.push({ index, status: outcome.status === "rejected" ? "failed" : outcome.status, message: outcome.message });
      }
      const applied = p.changes.filter((c) => c.outcome?.status === "applied").length;
      if (items.some((i) => i.status === "applied")) {
        s.last = { at: stamp(), provider: p.provider, trigger: "manual", outcome: { kind: "applied", count: applied } };
      }
      let undo: string | null = null;
      if (written.length) {
        undo = `${runId}#${undoCounter++}`;
        s.undo = { token: undo, runId, items: written };
      }
      changed();
      return { project, items, undo, updates: views() };
    },
    async undoUpdate(runId: string, token: string): Promise<UndoReport> {
      const entry = [...states.entries()].find(([, s]) => s.undo?.runId === runId && s.undo.token === token);
      if (!entry) throw new Error("Tej zmiany nie można już cofnąć: została cofnięta albo zastąpiona.");
      const [project, s] = entry;
      const record = s.undo!;
      s.undo = null;
      const items: UndoResult[] = [];
      // Newest write first; a full undo puts the topic's old date back while nothing else wrote it.
      for (const it of [...record.items].reverse()) {
        const current = it.updated ? vault.updatedOf(project, it.updated.topic) : null;
        let status: UndoResult["status"] = "changed";
        let message = "Zmieniło się w międzyczasie; zostawione bez zmian.";
        if (it.kind === "rejected") {
          const sc = s.proposal?.run_id === runId ? s.proposal.changes[it.index] : undefined;
          if (sc?.outcome?.status === "rejected") [status, message] = ["undone", "Przywrócono propozycję."];
          else message = "Propozycja nie jest już odrzucona.";
        } else if (it.kind === "task_status") {
          if (await vault.setTaskStatusIf(it.task, it.to, it.from)) [status, message] = ["undone", "Przywrócono poprzedni status."];
        } else if (it.kind === "new_topic") {
          if (await vault.deleteTopicIf(project, it.topic, it.stamp)) [status, message] = ["undone", "Usunięto dodany temat."];
        } else if (it.kind === "new_task") {
          const ok = await vault.deleteTaskIf(
            it.task,
            (t) => t.title === it.title && t.status === "todo" && !t.notes && !t.due && t.links.length === 0 && JSON.stringify(t.location) === JSON.stringify(it.location),
          );
          if (ok) [status, message] = ["undone", "Usunięto dodane zadanie."];
        } else {
          const r = await vault.revertJournal(project, it.topic, it.entry, it.status, it.summary);
          const wanted = 1 + (it.status ? 1 : 0) + (it.summary ? 1 : 0);
          const got = (r.entry ? 1 : 0) + (r.status ? 1 : 0) + (r.summary ? 1 : 0);
          if (got === wanted) [status, message] = ["undone", "Usunięto wpis z dziennika."];
          else if (got > 0) [status, message] = ["partial", "Cofnięto częściowo; reszta zmieniła się w międzyczasie."];
        }
        if (status === "undone" && it.updated && current === it.updated.written) vault.setUpdated(project, it.updated.topic, it.updated.before);
        items.unshift({ index: it.index, status, message });
        const sc = s.proposal?.run_id === runId ? s.proposal.changes[it.index] : undefined;
        if (sc && status === "undone") sc.outcome = null;
      }
      if (s.proposal) {
        const applied = s.proposal.changes.filter((c) => c.outcome?.status === "applied").length;
        const pending = s.proposal.changes.filter((c) => !c.outcome).length;
        s.last = { at: stamp(), provider: s.proposal.provider, trigger: "manual", outcome: applied ? { kind: "applied", count: applied } : { kind: "proposed", count: pending } };
      }
      changed();
      return { project, items, updates: views() };
    },
    /** Mirrors `store::edit_change`: the same checks as agent text, the base kept. */
    async editUpdate(runId: string, index: number, edit: ChangeEdit): Promise<ProjectUpdateView[]> {
      const s = [...states.entries()].find(([, st]) => st.proposal?.run_id === runId);
      if (!s) throw new Error("Tej propozycji już nie ma: została odrzucona albo zastąpiona nowszą.");
      const [project, st] = s;
      const p = st.proposal!;
      const sc = p.changes[index];
      if (!sc) throw new Error("Nie ma takiej propozycji.");
      if (sc.outcome) throw new Error("Tej zmiany nie można już poprawić: została rozstrzygnięta.");
      const snap = vault.snapshot();
      const stale = staleReason(sc.change, snap, project);
      if (stale) throw new Error(`Nie można już poprawić tej zmiany: ${stale}.`);
      const line = (field: string, v: string) => {
        const t = v.replace(/\s+/g, " ").trim();
        if (!t) throw new Error(`Pole „${field}” nie może być puste.`);
        if (t.length > 200) throw new Error(`Pole „${field}” jest za długie (limit 200 znaków).`);
        return t;
      };
      const text = (field: string, v: string, max: number) => {
        const t = v.trim();
        if (!t) throw new Error(`Pole „${field}” nie może być puste.`);
        if (t.length > max) throw new Error(`Pole „${field}” jest za długie (limit ${max} znaków).`);
        return t;
      };
      const others = p.changes.filter((_, i) => i !== index).map((c) => c.change);
      const c = sc.change;
      let next: StoredChange["change"];
      if (c.kind === "new_topic" && edit.kind === "new_topic") {
        if (edit.tasks.length !== c.tasks.length) throw new Error("Lista zadań tematu nie zgadza się z propozycją.");
        const kept = edit.tasks.filter((t) => t.keep).map((t) => ({ title: line("zadanie", t.title), status: t.status }));
        if (kept.some((t) => t.status === "blocked")) throw new Error("Zadanie nowego tematu może mieć status todo, doing albo done.");
        if (new Set(kept.map((t) => fold(t.title))).size !== kept.length) throw new Error("Powtórzone zadanie w temacie.");
        const title = line("tytuł", edit.title);
        const slug = slugify(title);
        if (snap.topics.some((t) => t.project === project && (t.slug === slug || fold(t.title) === fold(title)))) throw new Error("Taki temat już istnieje.");
        if (others.some((o) => o.kind === "new_topic" && (o.slug === slug || fold(o.title) === fold(title)))) throw new Error("Taki temat jest już w innej propozycji tej listy.");
        next = { ...c, title, slug, status: edit.status, summary: text("opis stanu", edit.summary, 4000), text: text("wpis", edit.text, 4000), tasks: kept };
      } else if (c.kind === "topic_log" && edit.kind === "topic_log") {
        const summary = edit.summary.trim();
        next = {
          ...c,
          text: text("wpis", edit.text, 4000),
          status: edit.status && edit.status !== c.base_status ? edit.status : null,
          summary: summary && summary !== c.base_summary ? summary : null,
        };
      } else if (c.kind === "task_status" && edit.kind === "task_status") {
        if (edit.status === c.from) throw new Error("Zadanie ma już ten status.");
        next = { ...c, to: edit.status };
      } else if (c.kind === "new_task" && edit.kind === "new_task") {
        const title = line("tytuł", edit.title);
        const loc: TaskLocation = c.topic ? { kind: "topic", project, topic: c.topic } : { kind: "project", project };
        if (snap.tasks.some((t) => JSON.stringify(t.location) === JSON.stringify(loc) && fold(t.title) === fold(title))) throw new Error("Takie zadanie już istnieje w tym miejscu.");
        if (others.some((o) => o.kind === "new_task" && o.topic === c.topic && fold(o.title) === fold(title))) throw new Error("Takie zadanie jest już w innej propozycji tej listy.");
        next = { ...c, title };
      } else throw new Error("Ta poprawka nie pasuje do rodzaju zmiany.");
      if (JSON.stringify(next) !== JSON.stringify(c)) {
        sc.change = next;
        sc.edited = true;
      }
      changed();
      return views();
    },
    /** Mirrors `store::join_change`: a new topic becomes an entry and tasks of an existing topic. */
    async joinUpdate(runId: string, index: number, topicSlug: string): Promise<JoinReport> {
      const s = [...states.entries()].find(([, st]) => st.proposal?.run_id === runId);
      if (!s) throw new Error("Tej propozycji już nie ma: została odrzucona albo zastąpiona nowszą.");
      const [project, st] = s;
      const p = st.proposal!;
      const sc = p.changes[index];
      if (!sc) throw new Error("Nie ma takiej propozycji.");
      if (sc.outcome) throw new Error("Tej zmiany nie można już zmienić: została rozstrzygnięta.");
      const c = sc.change;
      if (c.kind !== "new_topic") throw new Error("Dołączyć można tylko propozycję nowego tematu.");
      const snap = vault.snapshot();
      const topic = snap.topics.find((t) => t.project === project && t.slug === topicSlug);
      if (!topic) throw new Error("W tym projekcie nie ma takiego tematu.");
      if (p.changes.some((x, i) => i !== index && x.change.kind === "topic_log" && x.change.topic === topicSlug)) {
        throw new Error(`Na liście jest już wpis do tematu „${topic.title}”: popraw tamten wpis zamiast dołączać.`);
      }
      const skipped: string[] = [];
      const tasks: StoredChange[] = [];
      for (const t of c.tasks) {
        const exists = snap.tasks.some((x) => x.location.kind === "topic" && x.location.project === project && x.location.topic === topicSlug && fold(x.title) === fold(t.title));
        if (exists) skipped.push(t.title);
        else tasks.push({ change: { kind: "new_task", title: t.title, topic: topicSlug, topic_title: topic.title, reason: c.reason }, outcome: null, edited: true });
      }
      p.changes[index] = {
        change: { kind: "topic_log", topic: topicSlug, topic_title: topic.title, text: c.text, status: null, summary: null, base_status: topic.status, base_summary: topic.summary, branches: c.branches },
        outcome: null,
        edited: true,
      };
      p.changes.push(...tasks);
      changed();
      return { skipped, updates: views() };
    },
    /** Mirrors `dismiss_locked`: the rejection becomes the undoable last action. */
    async dismissUpdate(runId: string): Promise<DismissReport> {
      let count = 0;
      let undo: string | null = null;
      for (const s of states.values()) {
        if (s.proposal?.run_id !== runId) continue;
        const items: UndoItem[] = [];
        s.proposal.changes.forEach((c, index) => {
          if (c.outcome) return;
          c.outcome = { status: "rejected", message: "Odrzucone.", at: stamp() };
          items.push({ index, kind: "rejected" });
        });
        count = items.length;
        if (count) {
          undo = `${runId}#${undoCounter++}`;
          s.undo = { token: undo, runId, items };
        }
      }
      changed();
      return { count, undo, updates: views() };
    },
    async setAutoUpdate(project: string, config: AutoUpdate): Promise<ProjectUpdateView[]> {
      const s = config.schedule;
      if (s.kind === "every" && !(Number.isInteger(s.hours) && s.hours >= 1 && s.hours <= 168)) throw new Error(`Nieprawidłowy odstęp: ${s.hours} h (dozwolone 1–168).`);
      if (s.kind === "daily" && !(Number.isInteger(s.hour) && s.hour >= 0 && s.hour <= 23)) throw new Error(`Nieprawidłowa godzina: ${s.hour}.`);
      if (!vault.snapshot().projects.some((p) => p.slug === project)) throw new Error(`Nie ma projektu „${project}”.`);
      if (JSON.stringify(config) === JSON.stringify(DEFAULT_AUTO)) auto.delete(project);
      else auto.set(project, structuredClone(config));
      changed();
      return views();
    },
    async setUpdateSources(project: string, src: UpdateSources): Promise<ProjectUpdateView[]> {
      if (![7, 14, 30].includes(src.history_days)) throw new Error(`Nieprawidłowe okno historii: ${src.history_days} dni (dozwolone: 7, 14, 30).`);
      if (!vault.snapshot().projects.some((p) => p.slug === project)) throw new Error(`Nie ma projektu „${project}”.`);
      if (JSON.stringify(src) === JSON.stringify(DEFAULT_SOURCES)) sources.delete(project);
      else sources.set(project, structuredClone(src));
      changed();
      return views();
    },
    /** For the settings mock: the current sources map. */
    sourcesMap(): Record<string, UpdateSources> {
      return Object.fromEntries([...sources.entries()].map(([k, v]) => [k, structuredClone(v)]));
    },
    async onUpdatesChanged(cb: (u: ProjectUpdateView[]) => void) {
      listeners.add(cb);
      return () => void listeners.delete(cb);
    },
    /** For the settings mock: the current auto map. */
    autoMap(): Record<string, AutoUpdate> {
      return Object.fromEntries([...auto.entries()].map(([k, v]) => [k, structuredClone(v)]));
    },
  };
}

// The command field in the browser mock. The real reading comes from
// montybrain_core::command in Rust; the mock does NOT parse the command
// language. It answers a fixed set of example texts (the URL states below, for
// screenshots) with hand-written previews in the backend's shape, and treats
// any other text as a plain inbox capture of that exact title.
//
// URL states (main window `?cmd=…`, see views/InboxView.tsx and quick/):
//   reading   field with the live reading and autocomplete open
//   confirm   the confirm step with the plan
//   ambiguous a name that fits two topics: choices, nothing written
//   typo      "literówka" offered, still to be confirmed
//   similar   the duplicate hint while typing
//   topic     /temat from three selected entries (`?select=` is set too)
//   agent     a command inserted by the agent for one entry
//   echo      the echo after a write
//   problem   a text with a problem: the reading is the problem, no plan
//   partial   a failed write whose rollback could not finish (what stays written, undo)
//   refs      `/czeka` followed by plain words: tasks and topics by name, matches marked
//   pick      plain words naming several entries: the picker, nothing written
//   temat     `/temat Nazwa` still being typed: the projects to pick with ↑↓
//   typing    ` > ` with nothing after it: a neutral hint and the places, no error
// `?inbox=demo` adds entries, a duplicate topic slug and suggestions.

import { matchRanges, searchTerms } from "../lib/highlight";
import type {
  Captured,
  CommandApplyResult,
  CommandPlan,
  CommandPreview,
  CommandStep,
  CommandUndoReport,
  Completions,
  FieldInput,
  FieldPreview,
  InboxAgentAnswer,
  InboxSuggestion,
  NewTask,
  Place,
  Snapshot,
  Task,
  TaskLocation,
  Topic,
} from "./types";

export const MOCK_COMMAND_STATES = ["reading", "confirm", "ambiguous", "typo", "similar", "topic", "agent", "echo", "problem", "partial", "refs", "pick", "temat", "typing"] as const;
export type MockCommandState = (typeof MOCK_COMMAND_STATES)[number];

/** The text each state puts into the field (and the selection it needs). */
export const MOCK_COMMAND_TEXT: Record<MockCommandState, { text: string; selection?: string[] }> = {
  reading: { text: "Podpiąć zapisane karty > whenubuy" },
  confirm: { text: "Podpiąć zapisane karty !wysoki > whenubuy/platnosci-stripe-przelewy24" },
  ambiguous: { text: "Nagrać film instruktażowy > onboarding" },
  typo: { text: "Sprawdzić fakturę za hosting > whenbuy" },
  similar: { text: "Odpisać Kasi w sprawie faktury" },
  topic: { text: "/temat Łazienka > remont-kuchni", selection: ["d1e2m3", "d1e2m4", "d1e2m5"] },
  agent: { text: "^d1e2m6 > whenubuy/platnosci-stripe-przelewy24" },
  echo: { text: "Podpiąć zapisane karty !wysoki > whenubuy/platnosci-stripe-przelewy24" },
  problem: { text: "Spisać wymiary > remont-kuchni > whenubuy" },
  partial: { text: "> whenubuy/platnosci-stripe-przelewy24", selection: ["d1e2m6", "d1e2m7"] },
  refs: { text: "^d1e2m6 /czeka płat" },
  pick: { text: "łazienk /powiaz ^d1e2m7" },
  temat: { text: "/temat Łazienka" },
  typing: { text: "Kupić farbę > " },
};

/** `?inbox=demo`: more inbox entries and a topic slug used in two projects. */
export function addInboxDemo(db: Snapshot): void {
  if (db.tasks.some((t) => t.id === "d1e2m3")) return;
  const inbox: TaskLocation = { kind: "inbox" };
  const entry = (id: string, title: string, notes = ""): Task => ({
    id,
    title,
    status: "todo",
    priority: "normal",
    due: null,
    done: null,
    links: [],
    notes,
    location: inbox,
    line: 800 + db.tasks.length,
  });
  const firstNonInbox = db.tasks.findIndex((t) => t.location.kind !== "inbox");
  db.tasks.splice(
    firstNonInbox < 0 ? db.tasks.length : firstNonInbox,
    0,
    entry("d1e2m6", "Webhook zwrotów zwraca 500 na produkcji", "Od wczoraj, tylko dla BLIK."),
    entry("d1e2m3", "Płytki do łazienki: wycena z dwóch sklepów"),
    entry("d1e2m4", "Fuga epoksydowa i silikon do łazienki"),
    entry("d1e2m5", "Hydraulik: przesunięcie odpływu w łazience", "Pan Marek, po 16."),
    entry("d1e2m7", "Opisać instalację serwera MCP w README"),
  );
  if (!db.topics.some((t) => t.project === "montybrain" && t.slug === "onboarding")) {
    const t: Topic = {
      project: "montybrain",
      slug: "onboarding",
      title: "Onboarding w aplikacji",
      status: "planned",
      repo: null,
      summary: "Pierwsze uruchomienie: folder danych i serwer MCP.",
      created: null,
      updated: null,
      path: "montybrain/tematy/onboarding.md",
    };
    db.topics.push(t);
  }
}

interface Deps {
  snapshot: () => Snapshot;
  createTask: (location: TaskLocation, input: NewTask) => Promise<Task>;
  moveTask: (id: string, to: TaskLocation) => Promise<Task>;
  deleteTask: (id: string) => Promise<Task>;
  createTopic: (project: string, title: string) => Promise<Topic>;
  deleteTopic: (project: string, slug: string) => Promise<Topic>;
  today: () => string;
}

/** A stand-in for the id the backend picks with the plan (the mock does not reproduce its derivation). */
function plannedId(title: string): string {
  let h = 2166136261;
  for (const c of title) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  return h.toString(36).padStart(6, "0").slice(-6);
}

function placeOf(snap: Snapshot, loc: TaskLocation): Place {
  if (loc.kind === "inbox") return { location: loc, label: "Skrzynka", text: "skrzynka" };
  const p = snap.projects.find((x) => x.slug === loc.project);
  if (loc.kind === "project") return { location: loc, label: p?.name ?? loc.project, text: loc.project };
  const t = snap.topics.find((x) => x.project === loc.project && x.slug === loc.topic);
  return { location: loc, label: `${p?.name ?? loc.project} / ${t?.title ?? loc.topic}`, text: `${loc.project}/${loc.topic}` };
}

const base = (text: string): CommandPreview => ({
  text,
  state: "empty",
  reading: "",
  plan: null,
  problems: [],
  warnings: [],
  choices: [],
  typo: false,
  similar: [],
  agent: null,
});

function rel(loc: TaskLocation): string {
  if (loc.kind === "inbox") return "skrzynka.md";
  if (loc.kind === "project") return `${loc.project}/projekt.md`;
  return `${loc.project}/tematy/${loc.topic}.md`;
}

function newTaskStep(snap: Snapshot, loc: TaskLocation, task: NewTask, extra: string[] = []): CommandStep {
  const parts = [task.title];
  if (task.priority === "high") parts.push("!wysoki");
  if (task.due) parts.push(`@${task.due}`);
  return {
    op: { kind: "create_task", location: loc, task },
    summary: [`Nowe zadanie „${task.title}” w: ${placeOf(snap, loc).label}`, ...extra].join(" · "),
    file: rel(loc),
    lines: [`- [ ] ${parts.join(" ")} ^${plannedId(task.title)}`],
  };
}

/** Marks like the backend's (the mock only finds plain substrings). */
function marks(label: string, typed: string) {
  return matchRanges(label, searchTerms(typed.replace(/[^\p{L}\p{N}]+/gu, " "))).map(([from, to]) => ({ from, to }));
}

function plan(steps: CommandStep[], confirm: boolean): CommandPlan {
  return { steps, confirm };
}

/** Hand-written readings for the example texts (what the Rust core answers). */
function canned(snap: Snapshot, input: FieldInput): CommandPreview | null {
  const text = input.text;
  const wh = (topic?: string): TaskLocation => (topic ? { kind: "topic", project: "whenubuy", topic } : { kind: "project", project: "whenubuy" });
  switch (text) {
    case MOCK_COMMAND_TEXT.reading.text: {
      const s = newTaskStep(snap, wh(), { title: "Podpiąć zapisane karty" });
      return { ...base(text), state: "confirm", reading: s.summary, plan: plan([s], true) };
    }
    case MOCK_COMMAND_TEXT.confirm.text: {
      const s = newTaskStep(snap, wh("platnosci-stripe-przelewy24"), { title: "Podpiąć zapisane karty", priority: "high" }, ["priorytet wysoki"]);
      return { ...base(text), state: "confirm", reading: s.summary, plan: plan([s], true) };
    }
    case MOCK_COMMAND_TEXT.ambiguous.text: {
      const from = text.indexOf("onboarding");
      const span = { from, to: from + "onboarding".length };
      return {
        ...base(text),
        state: "invalid",
        reading: "„onboarding” pasuje do kilku miejsc: montybrain/onboarding, whenubuy/onboarding. Wybierz jedno.",
        problems: ["„onboarding” pasuje do kilku miejsc: montybrain/onboarding, whenubuy/onboarding. Wybierz jedno."],
        choices: (["montybrain", "whenubuy"] as const).map((project) => {
          const label = placeOf(snap, { kind: "topic", project, topic: "onboarding" }).label;
          return { span, choice: { label, insert: `${project}/onboarding`, kind: "topic", typo: false }, marks: marks(label, "onboarding") };
        }),
      };
    }
    case MOCK_COMMAND_TEXT.typo.text: {
      const from = text.indexOf("whenbuy");
      const s = newTaskStep(snap, wh(), { title: "Sprawdzić fakturę za hosting" });
      return {
        ...base(text),
        state: "confirm",
        reading: s.summary,
        plan: plan([s], true),
        typo: true,
        warnings: ["„whenbuy” to pewnie literówka: WhenUBuy (whenubuy)."],
        choices: [{ span: { from, to: from + 7 }, choice: { label: "WhenUBuy", insert: "whenubuy", kind: "project", typo: true }, marks: [{ from: 0, to: 4 }, { from: 5, to: 8 }] }],
      };
    }
    case MOCK_COMMAND_TEXT.similar.text: {
      const s = newTaskStep(snap, { kind: "inbox" }, { title: text });
      const t = snap.tasks.find((x) => x.id === "a1b2c3");
      return {
        ...base(text),
        state: "capture",
        reading: s.summary,
        plan: plan([s], false),
        similar: t ? [{ id: "a1b2c3", title: t.title, place: placeOf(snap, t.location), score: 0.75 }] : [],
      };
    }
    case MOCK_COMMAND_TEXT.topic.text: {
      const ids = input.selection?.length ? input.selection : (MOCK_COMMAND_TEXT.topic.selection ?? []);
      const tasks = ids.map((id) => snap.tasks.find((t) => t.id === id)).filter((t): t is Task => !!t);
      const lines = ["# Łazienka", "", "## Zadania"];
      for (const t of tasks) {
        lines.push(`- [ ] ${t.title} ^${t.id}`);
        if (t.notes) lines.push(...t.notes.split("\n").map((l) => `  ${l}`));
      }
      const summary = `Nowy temat „Łazienka” w: Remont kuchni · przeniesie do niego ${tasks.length} ${tasks.length === 1 ? "zadanie" : tasks.length < 5 ? "zadania" : "zadań"} (z identyfikatorami i notatkami)`;
      const step: CommandStep = {
        op: {
          kind: "create_topic",
          project: "remont-kuchni",
          title: "Łazienka",
          slug: "lazienka",
          summary: "",
          tasks: tasks.map((t) => ({ id: t.id!, from: t.location, fields: { status: t.status, priority: t.priority, due: t.due } })),
        },
        summary,
        file: "remont-kuchni/tematy/lazienka.md",
        lines,
      };
      return { ...base(text), state: "confirm", reading: summary, plan: plan([step], true) };
    }
    case MOCK_COMMAND_TEXT.problem.text: {
      const p = "Miejsce docelowe (` > `) może być tylko jedno.";
      return { ...base(text), state: "invalid", reading: p, problems: [p] };
    }
    case MOCK_COMMAND_TEXT.partial.text: {
      const ids = input.selection?.length ? input.selection : (MOCK_COMMAND_TEXT.partial.selection ?? []);
      const to = wh("platnosci-stripe-przelewy24");
      const steps = ids
        .map((id) => snap.tasks.find((t) => t.id === id))
        .filter((t): t is Task => !!t)
        .map((t) => ({
          op: { kind: "edit_task" as const, id: t.id!, title: t.title, from: t.location, to, before: null, after: null },
          summary: `„${t.title}”: przeniesie z: Skrzynka do: ${placeOf(snap, to).label}`,
          file: rel(to),
          lines: [`- [ ] ${t.title} ^${t.id}`],
        }));
      const reading = `Przeniesie ${steps.length} zadania do: ${placeOf(snap, to).label}`;
      return { ...base(text), state: "confirm", reading, plan: plan(steps, true) };
    }
    case MOCK_COMMAND_TEXT.pick.text: {
      const span = { from: 0, to: "łazienk".length };
      const p = "„łazienk” pasuje do kilku zadań i tematów. Wybierz jedno.";
      const choices = ["d1e2m4", "d1e2m5", "d1e2m3"]
        .map((id) => snap.tasks.find((t) => t.id === id))
        .filter((t): t is Task => !!t)
        .map((t) => {
          const label = `${t.title} · ${placeOf(snap, t.location).label}`;
          return { span, choice: { label, insert: `^${t.id}`, kind: "task", typo: false }, marks: marks(label, "łazienk") };
        });
      return { ...base(text), state: "invalid", reading: p, problems: [p], choices };
    }
    case MOCK_COMMAND_TEXT.temat.text: {
      const p = "Temat zakłada się w projekcie: dopisz ` > projekt`.";
      const end = { from: text.length, to: text.length };
      const choices = snap.projects
        .filter((x) => x.state !== "archived")
        .map((x) => ({ span: end, choice: { label: x.name, insert: ` > ${x.slug}`, kind: "project", typo: false } }));
      return { ...base(text), state: input.caret != null ? "typing" : "invalid", reading: input.caret != null ? "W którym projekcie? Dopisz „ > projekt” albo wybierz z listy" : p, problems: [p], choices };
    }
    case MOCK_COMMAND_TEXT.refs.text: {
      const p = "„płat” pasuje do kilku zadań i tematów. Wybierz jedno.";
      return { ...base(text), state: input.caret != null ? "typing" : "invalid", reading: input.caret != null ? "Na co czeka? Wpisz nazwę zadania albo tematu i wybierz z listy (albo ^id, [[projekt/temat]])" : p, problems: [p] };
    }
    case MOCK_COMMAND_TEXT.typing.text: {
      const p = "Po ` > ` wpisz projekt albo temat, np. `> whenubuy/checkout`.";
      return { ...base(text), state: input.caret != null ? "typing" : "invalid", reading: input.caret != null ? "Dokąd? Wpisz projekt albo temat i wybierz z listy" : p, problems: [p] };
    }
    case MOCK_COMMAND_TEXT.agent.text: {
      const t = snap.tasks.find((x) => x.id === "d1e2m6");
      if (!t) return null;
      const to = wh("platnosci-stripe-przelewy24");
      const summary = `„${t.title}”: przeniesie z: Skrzynka do: ${placeOf(snap, to).label}`;
      const step: CommandStep = {
        op: { kind: "edit_task", id: "d1e2m6", title: t.title, from: t.location, to, before: null, after: null },
        summary,
        file: rel(to),
        lines: [`- [ ] ${t.title} ^d1e2m6`, `  ${t.notes}`],
      };
      return { ...base(text), state: "confirm", reading: summary, plan: plan([step], true) };
    }
  }
  return null;
}

function cannedCompletions(snap: Snapshot, input: FieldInput): Completions | null {
  if (input.caret !== input.text.length) return null;
  if (input.text === MOCK_COMMAND_TEXT.refs.text) {
    const from = input.text.indexOf("płat");
    const typed = "płat";
    const items = [
      ...snap.topics
        .filter((t) => t.status !== "done" && /płat/i.test(t.title))
        .map((t) => {
          const pname = snap.projects.find((x) => x.slug === t.project)?.name ?? t.project;
          return { label: t.title, insert: `[[${t.project}/${t.slug}]]`, detail: `${pname} · [[${t.project}/${t.slug}]]`, kind: "topic", marks: marks(t.title, typed) };
        }),
      ...snap.tasks
        .filter((t) => t.status !== "done" && t.id && /płat/i.test(t.title))
        .map((t) => ({ label: t.title, insert: `^${t.id}`, detail: placeOf(snap, t.location).label, kind: "task", marks: marks(t.title, typed) })),
    ].slice(0, 8);
    return items.length ? { span: { from, to: from + typed.length }, kind: "ref", items } : null;
  }
  if (input.text === MOCK_COMMAND_TEXT.typing.text) {
    const from = input.text.length;
    const items = [
      ...snap.projects.filter((p) => p.state !== "archived").slice(0, 3).map((p) => ({ label: p.name, insert: p.slug, detail: "projekt", kind: "project" })),
      ...snap.topics.filter((t) => t.status !== "done").slice(0, 5).map((t) => ({ label: placeOf(snap, { kind: "topic", project: t.project, topic: t.slug }).label, insert: `${t.project}/${t.slug}`, detail: "temat", kind: "topic" })),
    ];
    return { span: { from, to: from }, kind: "place", items };
  }
  if (input.text !== MOCK_COMMAND_TEXT.reading.text) return null;
  const from = input.text.indexOf("whenubuy");
  const topics = snap.topics.filter((t) => t.project === "whenubuy" && t.status !== "done");
  return {
    span: { from, to: from + 8 },
    kind: "place",
    items: [
      { label: "WhenUBuy", insert: "whenubuy", detail: "projekt", kind: "project", marks: marks("WhenUBuy", "whenubuy") },
      ...topics.map((t) => ({ label: `WhenUBuy / ${t.title}`, insert: `whenubuy/${t.slug}`, detail: "temat", kind: "topic", marks: marks(`WhenUBuy / ${t.title}`, "whenubuy") })),
    ],
  };
}

export function createMockCommand(deps: Deps) {
  const undos = new Map<number, { reading: string; source: string; run: () => Promise<void> }>();
  let next = 1;

  function preview(input: FieldInput): CommandPreview {
    const snap = deps.snapshot();
    const text = input.text;
    if (!text.trim()) return base(text);
    const c = canned(snap, input);
    if (c) return c;
    if (text.trimStart().startsWith("?")) {
      return { ...base(text), state: "agent", reading: "Prośba do agenta (nic nie zapisze bez twojego potwierdzenia)", agent: { verb: "free", text: text.trim().slice(1).trim(), ids: [] } };
    }
    // Not an example: the mock does not parse commands, so the whole text is a title.
    const loc = input.location ?? { kind: "inbox" };
    const s = newTaskStep(snap, loc, { title: text.trim() });
    return { ...base(text), state: "capture", reading: s.summary, plan: plan([s], false) };
  }

  return {
    async commandPreview(input: FieldInput): Promise<FieldPreview> {
      return { preview: preview(input), completions: cannedCompletions(deps.snapshot(), input) };
    },

    async commandApply(input: FieldInput, expected: CommandPlan): Promise<CommandApplyResult> {
      const p = preview(input);
      if (!p.plan) return { kind: "refused", message: p.problems.join(" ") || "Nie ma czego zapisać.", preview: p };
      if (JSON.stringify(p.plan) !== JSON.stringify(expected)) {
        return { kind: "stale", message: "Dane zmieniły się od podglądu. Sprawdź plan jeszcze raz.", preview: p };
      }
      const echo: { file: string; line: number; text: string }[] = [];
      const undoRuns: (() => Promise<void>)[] = [];
      for (const step of p.plan.steps) {
        const op = step.op;
        if (op.kind === "create_task") {
          const t = await deps.createTask(op.location, op.task);
          echo.push({ file: step.file, line: t.line, text: step.lines[0]!.replace(/\^[0-9a-z]{6}$/, `^${t.id}`) });
          undoRuns.push(async () => void (await deps.deleteTask(t.id!)));
        } else if (op.kind === "edit_task") {
          await deps.moveTask(op.id, op.to);
          echo.push(...step.lines.map((text, i) => ({ file: step.file, line: 12 + i, text })));
          const back = op.from;
          undoRuns.push(async () => void (await deps.moveTask(op.id, back)));
        } else if (op.kind === "create_topic") {
          const t = await deps.createTopic(op.project, op.title);
          for (const e of op.tasks) await deps.moveTask(e.id, { kind: "topic", project: op.project, topic: t.slug });
          echo.push({ file: step.file, line: 6, text: `# ${op.title}` });
          echo.push(...step.lines.filter((l) => l.startsWith("- [")).map((text, i) => ({ file: step.file, line: 10 + i, text })));
          undoRuns.push(async () => {
            for (const e of op.tasks) await deps.moveTask(e.id, e.from);
            await deps.deleteTopic(op.project, t.slug);
          });
        }
      }
      const token = next++;
      undos.set(token, { reading: p.reading, source: input.source ?? "field", run: async () => { for (const u of undoRuns.reverse()) await u(); } });
      return { kind: "written", reading: p.reading, applied: { steps: p.plan.steps.map((s) => ({ summary: s.summary, ok: true, message: "" })), echo, error: null, remaining: [] }, undo: token };
    },

    async commandUndo(token: number | null, source: string | null): Promise<CommandUndoReport> {
      const keys = [...undos.entries()].filter(([, u]) => !source || u.source === source).map(([k]) => k);
      const key = token ?? Math.max(0, ...keys);
      const u = undos.get(key);
      if (!u) throw new Error("Nie ma już czego cofnąć.");
      await u.run();
      undos.delete(key);
      return { reading: u.reading, source: u.source, items: [{ status: "undone", message: "Cofnięto." }], retry: null };
    },

    async captureTask(location: TaskLocation, text: string): Promise<Captured> {
      const task = await deps.createTask(location, { title: text.trim() });
      return { task, undo: null };
    },

    async inboxSuggestions(): Promise<InboxSuggestion[]> {
      const snap = deps.snapshot();
      const to = (loc: TaskLocation, score: number) => ({ place: placeOf(snap, loc), score });
      const known: Record<string, InboxSuggestion["best"]> = {
        d1e2m6: to({ kind: "topic", project: "whenubuy", topic: "platnosci-stripe-przelewy24" }, 3.8),
        d1e2m7: to({ kind: "topic", project: "montybrain", topic: "serwer-mcp" }, 4.2),
        d1e2m3: to({ kind: "project", project: "remont-kuchni" }, 2.3),
        d1e2m4: to({ kind: "project", project: "remont-kuchni" }, 2.1),
        d1e2m5: to({ kind: "project", project: "remont-kuchni" }, 2.1),
        k9x2m1: null,
        a1b2c3: null,
      };
      return snap.tasks
        .filter((t) => t.location.kind === "inbox" && t.status !== "done" && t.id)
        .map((t) => {
          const best = known[t.id!] ?? null;
          const weak = t.id === "k9x2m1" ? [to({ kind: "project", project: "montybrain" }, 1.2), to({ kind: "project", project: "whenubuy" }, 1.0)] : [];
          return { id: t.id!, best, candidates: best ? [best] : weak };
        });
    },

    async inboxAgentAnswer(): Promise<InboxAgentAnswer> {
      return { command: MOCK_COMMAND_TEXT.agent.text, note: "Wpis dotyczy webhooka zwrotów, który jest w temacie płatności (Stripe → Przelewy24)." };
    },
  };
}

import { getLanguage } from "../i18n";
// In-memory stand-in for the Tauri backend, used when the page runs in a plain
// browser (`pnpm dev`) and in unit tests. It follows core's semantics where the
// UI can observe them: slugs, ids, done stamps, `today`, journal order, search.

import type { MontyBrainApi } from "./api";
import { journalRefreshRequest } from "../lib/journalAgent";
import { buildFixtures } from "./fixtures";
import { createMockRuns, type MockRunOptions } from "./mock-runs";
import { addMapDemo, buildLargeFixture } from "./mock-map";
import { createMockSignals } from "./mock-signals";
import { createMockUpdates, type UpdateScenario } from "./mock-updates";
import { addInboxDemo, createMockCommand } from "./mock-command";
import { DEFAULT_SHORTCUT, canonicalShortcut } from "./shortcut";
import { AGENT_EFFORTS } from "./types";
import type {
  AppSettings,
  AppView,
  JournalEntry,
  NewTask,
  Project,
  SearchHit,
  Snapshot,
  Task,
  TaskLocation,
  TodayItem,
  TodayReason,
  Topic,
  VaultChanged,
} from "./types";
import { addDays, toIsoDate, toIsoMinute } from "../lib/dates";
import { createMockSession, mockSessionView, type MockSessionState } from "./mock-session";
import { fold, oneLine, slugify, uniqueSlug } from "../lib/text";

export interface MockOptions {
  /** `?inbox=demo`: more inbox entries for the command field states. */
  inbox?: "demo";
  /** `?inboxagent=proposals`: an agent proposal waiting in Skrzynka. */
  inboxProposal?: boolean;
  /** Clock; defaults to the real one. */
  now?: () => Date;
  /** Initial data; defaults to the Polish fixtures. */
  snapshot?: Snapshot;
  /** Simulated IPC latency in ms. */
  latencyMs?: number;
  /** Initial settings (merged over the defaults). */
  settings?: Partial<AppSettings>;
  /** Agent-run mock (step delay, Codex availability). */
  runs?: MockRunOptions;
  /** Seeds one project-update state (screenshots; URL `?update=…`). */
  updateScenario?: UpdateScenario;
  /** `full`: extra fixture topics so every Dziś section shows (a waiting one, a stale one, a loose task in progress). */
  today?: "full";
  /** „Mapa" data: `demo` (references, a fourth project) or `large` (5 projects, 40 topics, 300 tasks). */
  map?: "demo" | "large";
  /** `none`: the drift detector has not collected anything yet. */
  drift?: "none" | "default";
  /** `?session=…`: an in-app agent session already on screen. */
  session?: MockSessionState;
}

/** Shortcuts the mock pretends are taken by the system (to exercise errors). */
export const MOCK_TAKEN_SHORTCUTS = ["Cmd+Space", "Ctrl+Space", "Cmd+Tab"];

const REASON_ORDER: TodayReason[] = ["overdue", "due_today", "doing", "blocked"];
const PRIORITY_RANK = { high: 0, normal: 1, low: 2 } as const;

/** Core's `Snapshot::today`, ported. */
export function computeToday(snap: Snapshot, today: string): TodayItem[] {
  const archived = new Set(snap.projects.filter((p) => p.state === "archived").map((p) => p.slug));
  const items: TodayItem[] = [];
  for (const task of snap.tasks) {
    if (task.status === "done") continue;
    if (task.location.kind !== "inbox" && archived.has(task.location.project)) continue;
    const dueDate = task.due?.slice(0, 10);
    let reason: TodayReason | null = null;
    if (dueDate && dueDate < today) reason = "overdue";
    else if (dueDate && dueDate === today) reason = "due_today";
    else if (task.status === "doing") reason = "doing";
    else if (task.status === "blocked") reason = "blocked";
    if (reason) items.push({ reason, task });
  }
  return items
    .map((it, i) => ({ it, i }))
    .sort((a, b) => {
      const r = REASON_ORDER.indexOf(a.it.reason) - REASON_ORDER.indexOf(b.it.reason);
      if (r) return r;
      const da = a.it.task.due;
      const db = b.it.task.due;
      if (da && db && da !== db) return da < db ? -1 : 1;
      if (da && !db) return -1;
      if (!da && db) return 1;
      return PRIORITY_RANK[a.it.task.priority] - PRIORITY_RANK[b.it.task.priority] || a.i - b.i;
    })
    .map(({ it }) => it);
}

/** Core's `Snapshot::recent_journal`, ported. */
export function computeRecentJournal(snap: Snapshot, limit: number): JournalEntry[] {
  return [...snap.journal]
    .sort(
      (a, b) =>
        b.at.localeCompare(a.at) ||
        a.project.localeCompare(b.project) ||
        a.topic.localeCompare(b.topic) ||
        a.line - b.line,
    )
    .slice(0, limit);
}

/** Core's `Snapshot::search`, ported. */
export function computeSearch(snap: Snapshot, query: string): SearchHit[] {
  const terms = fold(query).split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [];
  const hit = (...fields: string[]) => {
    const hay = fold(fields.join("\n"));
    return terms.every((t) => hay.includes(t));
  };
  return [
    ...snap.tasks.filter((t) => hit(t.title, t.notes)).map((task) => ({ kind: "task" as const, task })),
    ...snap.topics.filter((t) => hit(t.title, t.summary)).map((topic) => ({ kind: "topic" as const, topic })),
    ...snap.journal
      .filter((j) => hit(j.text, j.author))
      .sort((a, b) => b.at.localeCompare(a.at))
      .map((entry) => ({ kind: "journal" as const, entry })),
  ];
}

const TOKEN_TAIL = /(?:^|\s)(?:![a-ząćęłńóśźż]+|@\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2})?|✓\S*|\^[0-9a-z]{6})$/i;

function invalid(msg: string): Error {
  return new Error(`invalid input: ${msg}`);
}

function locationOrder(loc: TaskLocation): string {
  if (loc.kind === "inbox") return "0";
  if (loc.kind === "project") return `1/${loc.project}/0`;
  return `1/${loc.project}/1/${loc.topic}`;
}

const clone = <T>(v: T): T => structuredClone(v);

export function createMockApi(opts: MockOptions = {}): MontyBrainApi & { snapshotForTests(): Snapshot } {
  let nextJournalLine = 100_000;
  const now = opts.now ?? (() => new Date());
  const latency = opts.latencyMs ?? 0;
  const db: Snapshot = clone(opts.snapshot ?? (opts.map === "large" ? buildLargeFixture(toIsoDate(now())) : buildFixtures(toIsoDate(now()), toIsoMinute(now()))));
  if (opts.map === "demo") addMapDemo(db, toIsoDate(now()));
  if (opts.inbox === "demo" || opts.inboxProposal) addInboxDemo(db);
  const sessionMock = createMockSession(opts.session ? mockSessionView(opts.session) : null);
  let version = 1;
  const listeners = new Set<(e: VaultChanged) => void>();
  const settingsListeners = new Set<(s: AppSettings) => void>();
  const quickShowListeners = new Set<() => void>();
  let settings: AppSettings = {
    language: getLanguage(),
    shortcut: DEFAULT_SHORTCUT,
    shortcut_error: null,
    reminders: true,
    digest_hour: 9,
    autostart: false,
    autostart_error: null,
    quick_destination: { kind: "inbox" },
    auto_update: {},
    update_sources: {},
    agents: { claude: { model: null, effort: null }, codex: { model: null, effort: null } },
    agent_suggestions: { claude: ["opus", "sonnet", "fable"], codex: ["gpt-6.1-sol", "gpt-6-sol", "gpt-6-luna"] },
    waiting_days: 2,
    vault_path: db.root,
    config_path: "~/Library/Application Support/pl.montybrain.desktop",
    version: "0.1.0",
    ...opts.settings,
  };

  function settingsChanged(): AppSettings {
    const snap = clone({ ...settings, auto_update: updatesMock.autoMap(), update_sources: updatesMock.sourcesMap() });
    setTimeout(() => settingsListeners.forEach((cb) => cb(clone(snap))), 0);
    return snap;
  }

  if (opts.today === "full") {
    const day = (n: number) => addDays(toIsoDate(now()), n);
    const topic = (slug: string, project: string, title: string, status: Topic["status"], summary: string, updated: string): Topic => ({
      project,
      slug,
      title,
      status,
      repo: null,
      summary,
      created: day(-30),
      updated,
      path: `${project}/tematy/${slug}.md`,
    });
    db.topics.push(
      topic("wydanie-android", "whenubuy", "Wydanie Androida 2.5", "waiting", "Build 2.5.0 czeka na akceptację w Google Play.", `${day(-1)}T17:40`),
      topic("strona-cennik", "montybrain", "Strona z cennikiem", "in_progress", "Szkic sekcji z planami; brakuje porównania z konkurencją.", `${day(-10)}T09:00`),
    );
    db.journal.push(
      { project: "whenubuy", topic: "wydanie-android", at: `${day(-1)}T17:40`, author: "ja", text: "Czeka: akceptacja w Google Play", line: 901 },
      { project: "montybrain", topic: "strona-cennik", at: `${day(-10)}T09:00`, author: "claude-code", text: "Szkic sekcji z planami.", line: 902 },
    );
    db.tasks.push({ id: "v1a2t3", title: "Rozliczyć VAT z księgową", status: "doing", priority: "normal", due: null, done: null, links: [], notes: "", location: { kind: "project", project: "whenubuy" }, line: 903 });
  }

  // The `topics` scenario needs a project with repositories and nothing else yet.
  if (opts.updateScenario === "topics" && !db.projects.some((p) => p.slug === "beeking")) {
    db.projects.push({
      slug: "beeking",
      name: "Beeking",
      color: "#fbbf24",
      state: "active",
      repos: [
        { name: "beeking-app", path: "~/Projects/beeking/app" },
        { name: "beeking-api", path: "~/Projects/beeking/api" },
      ],
      description: "Sprzedaż miodu na targach i w sklepie internetowym.",
      path: "beeking/projekt.md",
    });
    db.projects.sort((a, b) => a.slug.localeCompare(b.slug));
  }
  /** What a topic looks like now (its file, as far as the mock can tell). */
  const topicStamp = (projectSlug: string, slug: string) =>
    JSON.stringify([
      db.topics.find((t) => t.project === projectSlug && t.slug === slug),
      db.tasks.filter((t) => t.location.kind === "topic" && t.location.project === projectSlug && t.location.topic === slug),
      db.journal.filter((j) => j.project === projectSlug && j.topic === slug),
    ]);

  const runsMock = createMockRuns({ now, agentFor: (p) => settings.agents[p], ...opts.runs });
  const signalsMock = createMockSignals({
    today: () => toIsoDate(now()),
    now: () => toIsoMinute(now()),
    snapshot: () => db,
    waitingDays: () => settings.waiting_days,
    drift: opts.drift,
  });
  const updatesMock = createMockUpdates(
    {
      snapshot: () => clone(db),
      setTaskStatus: (id, status) => api.updateTask(id, { status }),
      addJournal: (project, topic, author, text, status, summary) => api.addJournalEntry(project, topic, { author, text, status, summary }),
      createTask: (location, title) => api.createTask(location, { title }),
      moveTask: (id, to) => api.moveTask(id, to),
      async createTopicWith(projectSlug, input, author, text, tasks) {
        const t = await api.createTopic(projectSlug, { title: input.title, status: input.status, summary: input.summary, repo: input.repo });
        await api.addJournalEntry(projectSlug, t.slug, { author, text });
        for (const task of tasks) await api.createTask({ kind: "topic", project: projectSlug, topic: t.slug }, { title: task.title, status: task.status });
        return { slug: t.slug, stamp: topicStamp(projectSlug, t.slug) };
      },
      async deleteTopicIf(projectSlug, slug, stamp) {
        if (!db.topics.some((t) => t.project === projectSlug && t.slug === slug) || topicStamp(projectSlug, slug) !== stamp) return false;
        await api.deleteTopic(projectSlug, slug);
        return true;
      },
      async setTaskStatusIf(id, expected, status) {
        const t = db.tasks.find((x) => x.id === id);
        if (!t || t.status !== expected) return false;
        await api.updateTask(id, { status });
        return true;
      },
      async restoreTask(task, position) {
        if (db.tasks.some((t) => t.id === task.id)) return false;
        db.tasks.splice(position, 0, clone(task));
        changed([]);
        return true;
      },
      async deleteTaskIf(id, guard) {
        const t = db.tasks.find((x) => x.id === id);
        if (!t || !guard(clone(t))) return false;
        await api.deleteTask(id);
        return true;
      },
      link: (repo, branch, topic) => signalsMock.link(repo, branch, topic),
      updatedOf: (projectSlug, topicSlug) => db.topics.find((t) => t.project === projectSlug && t.slug === topicSlug)?.updated ?? null,
      setUpdated(projectSlug, topicSlug, value) {
        const t = db.topics.find((x) => x.project === projectSlug && x.slug === topicSlug);
        if (!t) return;
        t.updated = value;
        changed([t.path]);
      },
      async revertJournal(projectSlug, topicSlug, entry, status, summary) {
        const t = topic(projectSlug, topicSlug);
        const i = db.journal.findIndex((j) => j.project === projectSlug && j.topic === topicSlug && j.at === entry.at && j.author === entry.author && j.text === entry.text);
        const out = { entry: i >= 0, status: false, summary: false };
        if (i >= 0) db.journal.splice(i, 1);
        if (status && t.status === status[0]) {
          t.status = status[1];
          out.status = true;
        }
        if (summary && t.summary === summary[0]) {
          t.summary = summary[1];
          out.summary = true;
        }
        if (out.entry || out.status || out.summary) {
          t.updated = stamp();
          changed([t.path]);
        }
        return out;
      },
    },
    runsMock,
    { now, scenario: opts.updateScenario },
  );

  const wait = () => (latency > 0 ? new Promise((r) => setTimeout(r, latency)) : Promise.resolve());
  const today = () => toIsoDate(now());
  const stamp = () => toIsoMinute(now());

  function changed(paths: string[] = []) {
    version += 1;
    // File order, like core: inbox, then per project projekt.md before topics.
    db.tasks = db.tasks
      .map((t, i) => ({ t, i, k: locationOrder(t.location) }))
      .sort((a, b) => (a.k < b.k ? -1 : a.k > b.k ? 1 : a.i - b.i))
      .map(({ t }) => t);
    db.projects.sort((a, b) => a.slug.localeCompare(b.slug));
    db.topics.sort((a, b) => a.project.localeCompare(b.project) || a.slug.localeCompare(b.slug));
    const event: VaultChanged = { version, source: "mutation", paths };
    setTimeout(() => listeners.forEach((cb) => cb(event)), 0);
  }

  function project(slug: string): Project {
    const p = db.projects.find((p) => p.slug === slug);
    if (!p) throw new Error(`project not found: ${slug}`);
    return p;
  }

  function topic(projectSlug: string, slug: string): Topic {
    project(projectSlug);
    const t = db.topics.find((t) => t.project === projectSlug && t.slug === slug);
    if (!t) throw new Error(`topic not found: ${projectSlug}/${slug}`);
    return t;
  }

  function task(id: string): Task {
    const clean = id.replace(/^\^/, "");
    const t = db.tasks.find((t) => t.id === clean);
    if (!t) throw new Error(`task not found: ^${clean}`);
    return t;
  }

  function checkLocation(loc: TaskLocation) {
    if (loc.kind === "project") project(loc.project);
    if (loc.kind === "topic") topic(loc.project, loc.topic);
  }

  function touch(loc: TaskLocation) {
    if (loc.kind === "topic") topic(loc.project, loc.topic).updated = stamp();
  }

  function checkTitle(title: string, hasDue: boolean): string {
    const t = oneLine(title);
    if (!t) throw invalid("task title must not be empty");
    if (TOKEN_TAIL.test(t) && !(hasDue && /@\d{4}-/.test(t.split(" ").pop() ?? ""))) {
      throw invalid(`title "${t}" ends with something that reads as a token`);
    }
    return t;
  }

  function newId(): string {
    const taken = new Set(db.tasks.map((t) => t.id));
    for (;;) {
      const id = Array.from({ length: 6 }, () => "0123456789abcdefghijklmnopqrstuvwxyz"[Math.floor(Math.random() * 36)]).join("");
      if (!taken.has(id)) return id;
    }
  }

  function checkRepo(p: Project, repo: string | null | undefined) {
    if (repo && !p.repos.some((r) => r.name === repo)) throw invalid(`unknown repo "${repo}" for project ${p.slug}`);
  }

  const commandMock = createMockCommand({
    snapshot: () => db,
    createTask: (location, input) => api.createTask(location, input),
    moveTask: (id, to) => api.moveTask(id, to),
    deleteTask: (id) => api.deleteTask(id),
    createTopic: (project, title) => api.createTopic(project, { title }),
    deleteTopic: (project, slug) => api.deleteTopic(project, slug),
    today: () => toIsoDate(now()),
  });

  const api: MontyBrainApi & { snapshotForTests(): Snapshot } = {
    mode: "mock",
    snapshotForTests: () => clone(db),

    async getState(): Promise<AppView> {
      await wait();
      const snap = clone(db);
      return {
        version,
        today: today(),
        now: stamp(),
        snapshot: snap,
        today_items: computeToday(snap, today()),
        recent_journal: computeRecentJournal(snap, 30),
        load_error: null,
      };
    },

    async onChange(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },

    async search(query) {
      await wait();
      return computeSearch(clone(db), query);
    },

    async createProject(input) {
      await wait();
      const name = oneLine(input.name ?? "");
      if (!name) throw invalid("project name must not be empty");
      const slug = uniqueSlug(slugify(name), (s) => db.projects.some((p) => p.slug === s));
      const p: Project = {
        slug,
        name,
        color: input.color ?? null,
        state: input.state ?? "active",
        repos: input.repos ?? [],
        description: (input.description ?? "").trim(),
        path: `${slug}/projekt.md`,
      };
      db.projects.push(p);
      changed([p.path]);
      return clone(p);
    },

    async updateProject(slug, patch) {
      await wait();
      const p = project(slug);
      if (patch.name !== undefined) {
        const name = oneLine(patch.name);
        if (!name) throw invalid("project name must not be empty");
        p.name = name;
      }
      if (patch.color !== undefined) p.color = patch.color;
      if (patch.state !== undefined) p.state = patch.state;
      if (patch.repos !== undefined) p.repos = patch.repos;
      if (patch.description !== undefined) p.description = patch.description.trim();
      changed([p.path]);
      return clone(p);
    },

    async archiveProject(slug) {
      return api.updateProject(slug, { state: "archived" });
    },

    async createTopic(projectSlug, input) {
      await wait();
      const p = project(projectSlug);
      const title = oneLine(input.title ?? "");
      if (!title) throw invalid("topic title must not be empty");
      checkRepo(p, input.repo);
      const slug = uniqueSlug(slugify(title), (s) => db.topics.some((t) => t.project === p.slug && t.slug === s));
      const t: Topic = {
        project: p.slug,
        slug,
        title,
        status: input.status ?? "planned",
        repo: input.repo ?? null,
        summary: (input.summary ?? "").trim(),
        created: today(),
        updated: stamp(),
        path: `${p.slug}/tematy/${slug}.md`,
      };
      db.topics.push(t);
      changed([t.path]);
      return clone(t);
    },

    async updateTopic(projectSlug, slug, patch) {
      await wait();
      const t = topic(projectSlug, slug);
      if (patch.title !== undefined) {
        const title = oneLine(patch.title);
        if (!title) throw invalid("topic title must not be empty");
        t.title = title;
      }
      if (patch.status !== undefined) t.status = patch.status;
      if (patch.repo !== undefined) {
        checkRepo(project(projectSlug), patch.repo);
        t.repo = patch.repo;
      }
      if (patch.summary !== undefined) t.summary = patch.summary.trim();
      t.updated = stamp();
      changed([t.path]);
      return clone(t);
    },

    async deleteTopic(projectSlug, slug) {
      await wait();
      const t = topic(projectSlug, slug);
      db.topics = db.topics.filter((x) => x !== t);
      db.tasks = db.tasks.filter((x) => !(x.location.kind === "topic" && x.location.project === projectSlug && x.location.topic === slug));
      db.journal = db.journal.filter((j) => !(j.project === projectSlug && j.topic === slug));
      changed([t.path]);
      return clone(t);
    },

    async addJournalEntry(projectSlug, slug, entry) {
      await wait();
      const t = topic(projectSlug, slug);
      const text = entry.text.trim();
      if (!text) throw invalid("journal text must not be empty");
      const author = oneLine((entry.author ?? "").replace(/·/g, " ")) || "ja";
      const e: JournalEntry = { project: projectSlug, topic: slug, at: stamp(), author, text, line: nextJournalLine-- };
      db.journal.unshift(e);
      if (entry.status) t.status = entry.status;
      if (entry.summary != null) t.summary = entry.summary.trim();
      t.updated = stamp();
      changed([t.path]);
      return { topic: clone(t), entry: clone(e) };
    },

    async createTask(location, input: NewTask) {
      await wait();
      checkLocation(location);
      const title = checkTitle(input.title ?? "", !!input.due);
      const status = input.status ?? "todo";
      const t: Task = {
        id: newId(),
        title,
        status,
        priority: input.priority ?? "normal",
        due: input.due ?? null,
        done: status === "done" ? today() : null,
        links: [...new Set(input.links ?? [])],
        notes: (input.notes ?? "").trim(),
        location: clone(location),
        line: db.tasks.length + 1,
      };
      db.tasks.push(t);
      touch(location);
      changed();
      return clone(t);
    },

    async updateTask(id, patch) {
      await wait();
      const t = task(id);
      const due = patch.due !== undefined ? patch.due : t.due;
      if (patch.title !== undefined) t.title = checkTitle(patch.title, !!due);
      if (patch.status !== undefined && patch.status !== t.status) {
        t.status = patch.status;
        t.done = patch.status === "done" ? today() : null;
      }
      if (patch.priority !== undefined) t.priority = patch.priority;
      if (patch.due !== undefined) t.due = patch.due;
      for (const l of patch.add_links ?? []) if (!t.links.includes(l)) t.links.push(l);
      if (patch.remove_links?.length) t.links = t.links.filter((l) => !patch.remove_links!.includes(l));
      if (patch.notes !== undefined) t.notes = patch.notes.replace(/^\n+|\s+$/g, "");
      touch(t.location);
      changed();
      return clone(t);
    },

    async moveTask(id, to) {
      await wait();
      const t = task(id);
      checkLocation(to);
      if (locationOrder(t.location) === locationOrder(to)) return clone(t);
      touch(t.location);
      db.tasks = db.tasks.filter((x) => x !== t);
      t.location = clone(to);
      db.tasks.push(t);
      touch(to);
      changed();
      return clone(t);
    },

    async deleteTask(id) {
      await wait();
      const t = task(id);
      db.tasks = db.tasks.filter((x) => x !== t);
      touch(t.location);
      changed();
      return clone(t);
    },

    async openUrl(url) {
      window.open(url, "_blank", "noopener,noreferrer");
    },

    async revealPath(path) {
      console.info(`[mock] reveal ${db.root}/${path}`);
    },

    async revealVault() {
      console.info(`[mock] reveal ${db.root}`);
    },

    async getSettings() {
      await wait();
      return clone({ ...settings, auto_update: updatesMock.autoMap(), update_sources: updatesMock.sourcesMap() });
    },

    async updateSettings(patch) {
      if (patch.language !== undefined && patch.language !== "en" && patch.language !== "pl") throw new Error("Nieprawidłowa wartość języka.");
      await wait();
      if (patch.digest_hour !== undefined && !(Number.isInteger(patch.digest_hour) && patch.digest_hour >= 0 && patch.digest_hour <= 23)) {
        throw new Error(`Nieprawidłowa godzina: ${patch.digest_hour}`);
      }
      if (patch.quick_destination) checkLocation(patch.quick_destination);
      let agents = settings.agents;
      if (patch.agents) {
        agents = clone(patch.agents);
        for (const p of ["claude", "codex"] as const) {
          const c = agents[p];
          c.model = c.model?.trim() || null;
          c.effort = c.effort?.trim() || null;
          if (c.model && (c.model.length > 64 || c.model.startsWith("-") || !/^[A-Za-z0-9._:/@[\]-]+$/.test(c.model))) {
            throw new Error(`Nieprawidłowa nazwa modelu dla ${p === "claude" ? "Claude Code" : "Codex"}: „${c.model}”.`);
          }
          if (c.effort && !AGENT_EFFORTS[p].includes(c.effort)) throw new Error(`Nieznany poziom wysiłku: „${c.effort}”.`);
        }
      }
      settings = {
        ...settings,
        ...(patch.language !== undefined && { language: patch.language }),
        ...(patch.reminders !== undefined && { reminders: patch.reminders }),
        ...(patch.digest_hour !== undefined && { digest_hour: patch.digest_hour }),
        ...(patch.quick_destination && { quick_destination: clone(patch.quick_destination) }),
        ...(patch.waiting_days !== undefined && { waiting_days: patch.waiting_days }),
        agents,
      };
      return settingsChanged();
    },

    async setShortcut(shortcut) {
      await wait();
      const canonical = canonicalShortcut(shortcut);
      if (MOCK_TAKEN_SHORTCUTS.includes(canonical)) {
        throw new Error(`Nie udało się ustawić skrótu ${canonical}: jest zajęty przez inną aplikację lub system.`);
      }
      settings = { ...settings, shortcut: canonical, shortcut_error: null };
      return settingsChanged();
    },

    async pauseShortcut() {
      return clone({ ...settings, auto_update: updatesMock.autoMap(), update_sources: updatesMock.sourcesMap() });
    },

    async setAutostart(enabled) {
      await wait();
      settings = { ...settings, autostart: enabled };
      return settingsChanged();
    },

    async onSettingsChange(cb) {
      // A wrapper per subscription: the same callback subscribed twice (React
      // StrictMode in dev) must not share one entry that the first cleanup removes.
      const w = (x: AppSettings) => cb(x);
      settingsListeners.add(w);
      return () => settingsListeners.delete(w);
    },

    async quickClose() {
      console.info("[mock] quick window closed");
      setTimeout(() => quickShowListeners.forEach((cb) => cb()), 0);
    },

    async quickResize() {},

    ...commandMock,
    ...sessionMock,
    async inboxAgent(request) {
      await wait();
      const at = toIsoMinute(now()) + ":00";
      return {
        id: `mock-inbox-${Date.now()}`,
        recipe: request.mode === "command" && request.tools !== "mine" ? "skrzynka-komenda" : "skrzynka",
        recipe_title: request.mode === "command" && request.tools !== "mine" ? "Komenda dla wpisu" : "Porządek w Skrzynce",
        provider: request.provider,
        mode: request.tools === "mine" ? "terminal" : "background",
        project: null,
        request: request.text,
        model: null,
        effort: null,
        status: "succeeded",
        started_at: at,
        finished_at: at,
        result: null,
        error: null,
        events: [],
      };
    },

    async inboxAgentCheck() {
      await wait();
      return { status: "waiting" as const, count: 0 };
    },
    async inboxAgentPaste() {
      await wait();
      throw new Error("W odpowiedzi agenta nie ma propozycji do wczytania (podgląd w przeglądarce).");
    },

    listRecipes: runsMock.listRecipes,
    listProviders: runsMock.listProviders,
    async startRun(request) {
      if (request.params.topic) {
        if (!(await updatesMock.listUpdates()).every((v) => v.project !== request.params.project || v.pending === 0)) throw new Error("Projekt ma nierozstrzygnięte propozycje. Najpierw przejrzyj zmiany.");
        if (request.recipe !== "zlecenie" || request.mode !== "background") throw new Error("Odświeżanie tematu działa tylko jako zlecenie w tle.");
        topic(request.params.project!, request.params.topic);
        const input = updatesMock.prepareJournalRefresh(request.params.project!, request.params.topic);
        const run = await runsMock.startRun({ ...request, params: { ...request.params, text: journalRefreshRequest(input.entries) } });
        updatesMock.trackJournalRefresh(run.id, input);
        return run;
      }
      return runsMock.startRun(request);
    },
    cancelRun: runsMock.cancelRun,
    listRuns: runsMock.listRuns,
    onRunEvent: runsMock.onRunEvent,
    onRunUpdated: runsMock.onRunUpdated,

    listUpdates: updatesMock.listUpdates,
    applyUpdate: updatesMock.applyUpdate,
    undoUpdate: updatesMock.undoUpdate,
    dismissUpdate: updatesMock.dismissUpdate,
    async editUpdate(runId, index, edit) {
      await wait();
      return updatesMock.editUpdate(runId, index, edit);
    },
    async setAutoUpdate(project, config) {
      await wait();
      const out = await updatesMock.setAutoUpdate(project, config);
      settingsChanged();
      return out;
    },
    async setUpdateSources(project, sources) {
      await wait();
      const out = await updatesMock.setUpdateSources(project, sources);
      settingsChanged();
      return out;
    },
    onUpdatesChanged: updatesMock.onUpdatesChanged,
    async joinUpdate(runId, index, topic) {
      await wait();
      return updatesMock.joinUpdate(runId, index, topic);
    },
    listDrift: signalsMock.listDrift,
    refreshDrift: signalsMock.refreshDrift,
    linkBranch: signalsMock.linkBranch,
    ignoreBranch: signalsMock.ignoreBranch,
    onDriftChanged: signalsMock.onDriftChanged,
    listWaiting: signalsMock.listWaiting,
    setWaiting: signalsMock.setWaiting,
    confirmWaiting: signalsMock.confirmWaiting,
    onWaitingChanged: signalsMock.onWaitingChanged,

    async onQuickShow(cb) {
      quickShowListeners.add(cb);
      return () => quickShowListeners.delete(cb);
    },
  };
  if (opts.inboxProposal) {
    const at = toIsoMinute(now());
    updatesMock.seedProposal("", {
      run_id: opts.session === "idle" || opts.session === "finished" ? "sesja-mock" : "mock-inbox-proposal",
      provider: "claude",
      trigger: "manual",
      created_at: at,
      report: "Trzy wpisy dotyczą jednego remontu łazienki, więc proponuję jeden temat. Webhook zwrotów pasuje do płatności.",
      request: "?dopasuj",
      changes: [
        {
          change: {
            kind: "topic_from_tasks",
            project: "remont-kuchni",
            project_name: "Remont kuchni",
            title: "Łazienka",
            slug: "lazienka",
            summary: "Wycena płytek, fuga i hydraulik.",
            tasks: ["d1e2m3", "d1e2m4", "d1e2m5"].map((id) => ({ id, title: db.tasks.find((t) => t.id === id)?.title ?? id, from: { kind: "inbox" as const } })),
            reason: "Wszystkie trzy wpisy dotyczą remontu łazienki.",
          },
          outcome: null,
        },
        {
          change: {
            kind: "move_task",
            task: "d1e2m6",
            title: db.tasks.find((t) => t.id === "d1e2m6")?.title ?? "",
            from: { kind: "inbox" },
            to: { kind: "topic", project: "whenubuy", topic: "platnosci-stripe-przelewy24" },
            to_label: "WhenUBuy / Płatności: Stripe → Przelewy24",
            reason: "Webhook zwrotów jest częścią integracji płatności.",
          },
          outcome: null,
        },
      ],
      dropped: [],
      dropped_more: 0,
      problem: null,
    });
  }
  return api;
}


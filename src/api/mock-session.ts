// The in-app agent session in the browser mock: a scripted conversation (no
// agent runs). `pnpm dev` with "Moje narzędzia" plays it: a tool call, a
// permission card, the answer with proposals, follow-ups. URL states for
// screenshots: `?session=working|card|question|idle|failed` (the panel opens).

import type { ProviderId, SessionAnswer, SessionEntry, SessionStartRequest, SessionView } from "./types";

export const MOCK_SESSION_STATES = ["working", "card", "question", "idle", "failed"] as const;
export type MockSessionState = (typeof MOCK_SESSION_STATES)[number];

const at = "2026-11-14T10:00:00";

function base(provider: ProviderId = "claude"): SessionView {
  return {
    id: "sesja-mock",
    provider,
    scope: { kind: "inbox" },
    request: "Sprawdź na Slacku kanał #zespol i zaproponuj zadania do Skrzynki",
    status: "working",
    entries: [],
    started_at: at,
    ended_at: null,
    proposal: null,
    error: null,
    remembered: [],
  };
}

const intro: SessionEntry[] = [
  { kind: "user", id: 1, text: "Sprawdź na Slacku kanał #zespol i zaproponuj zadania do Skrzynki" },
  { kind: "notice", id: 2, text: "Claude Code połączony (model claude-fable).", error: false },
  { kind: "assistant", id: 3, text: "Najpierw przejrzę wpisy w Skrzynce, potem kanał #zespol na Slacku." },
  { kind: "tool", id: 4, name: "mcp__montybrain__overview", summary: "", detail: "", status: "done", output: "Projekty: 3, Skrzynka: 9 wpisów" },
  { kind: "tool", id: 5, name: "mcp__slack__read_channel", summary: "#zespol", detail: '{\n  "channel": "#zespol",\n  "limit": 50\n}', status: "running", output: "" },
];

const card: SessionEntry = {
  kind: "permission",
  id: 6,
  title: "slack - read_channel: #zespol",
  detail: '{\n  "channel": "#zespol",\n  "limit": 50\n}',
  tool: "mcp__slack__read_channel",
  state: "pending",
  can_session: true,
  note: "",
  questions: [],
};

const denied: SessionEntry = {
  kind: "permission",
  id: 7,
  title: "mcp__montybrain__create_task",
  detail: '{\n  "title": "Odpisać Kasi"\n}',
  tool: "mcp__montybrain__create_task",
  state: "auto_denied",
  can_session: false,
  note: "MontyBrain jest dla agenta tylko do odczytu: zmiany wracają jako propozycje do zaznaczenia.",
  questions: [],
};

const answer: SessionEntry[] = [
  { kind: "assistant", id: 9, text: "Na kanale #zespol są trzy sprawy dla ciebie: faktura za hosting, review MR w płatnościach i spotkanie w czwartek. Proponuję dwa zadania do Skrzynki i przeniesienie wpisu o fakturze do WhenUBuy." },
  { kind: "notice", id: 10, text: "Propozycje (3) czekają w Skrzynce: zaznacz, co zapisać.", error: false },
];

/** A ready view for a URL state. */
export function mockSessionView(state: MockSessionState): SessionView {
  const v = base();
  switch (state) {
    case "working":
      return { ...v, entries: intro };
    case "card":
      return { ...v, status: "needs_you", entries: [...intro, denied, card] };
    case "question":
      return {
        ...base("codex"),
        status: "needs_you",
        entries: [
          { kind: "user", id: 1, text: v.request },
          { kind: "notice", id: 2, text: "Codex połączony (model gpt-5-codex).", error: false },
          { kind: "tool", id: 3, name: "computer-use/screenshot", summary: "Slack", detail: '{\n  "app": "Slack"\n}', status: "done", output: "[image]" },
          {
            kind: "permission",
            id: 4,
            title: "Który kanał sprawdzić?",
            detail: "",
            tool: "question",
            state: "pending",
            can_session: false,
            note: "",
            questions: [{ id: "k", header: "Kanał", question: "Który kanał sprawdzić?", options: ["#zespol", "#ogolny", "#platnosci"] }],
          },
        ],
      };
    case "idle":
      return {
        ...v,
        status: "idle",
        entries: [...intro.slice(0, 4), { ...(intro[4] as Extract<SessionEntry, { kind: "tool" }>), status: "done", output: "50 wiadomości" }, denied, { ...(card as Extract<SessionEntry, { kind: "permission" }>), state: "allowed_session" }, ...answer],
        proposal: { count: 3, at },
        remembered: ["mcp__slack__read_channel"],
      };
    case "failed":
      return { ...v, status: "failed", entries: intro.slice(0, 2), ended_at: at, error: "Claude Code: nie jesteś zalogowany. Uruchom `claude` w Terminalu i zaloguj się." };
  }
}

export function createMockSession(initial: SessionView | null) {
  let view: SessionView | null = initial;
  const subs = new Set<(v: SessionView) => void>();
  let timers: ReturnType<typeof setTimeout>[] = [];
  let next = 100;
  const emit = () => view && subs.forEach((f) => f(structuredClone(view!)));
  const later = (ms: number, f: () => void) => timers.push(globalThis.setTimeout(f, ms));
  const add = (e: SessionEntry) => {
    if (!view) return;
    view = { ...view, entries: [...view.entries, e] };
  };
  const set = (patch: Partial<SessionView>) => {
    if (view) view = { ...view, ...patch };
  };
  const live = () => !!view && ["starting", "working", "needs_you", "idle"].includes(view.status);

  return {
    async agentSessionStart(r: SessionStartRequest): Promise<SessionView> {
      if (live()) throw new Error("Sesja agenta już trwa: zakończ ją, zanim zaczniesz nową.");
      timers.forEach((t) => globalThis.clearTimeout(t));
      timers = [];
      const request = r.text.trim() || "Porządek w Skrzynce";
      view = { ...base(r.provider), request, scope: r.scope === "project" ? { kind: "project", project: r.project ?? "" } : { kind: "inbox" }, status: "starting", entries: [{ kind: "user", id: 1, text: request }] };
      const name = r.provider === "claude" ? "Claude Code" : "Codex";
      later(400, () => (add({ kind: "notice", id: 2, text: `${name} połączony (podgląd w przeglądarce).`, error: false }), set({ status: "working" }), emit()));
      later(1100, () => (add(intro[2]!), add(intro[3]!), emit()));
      later(1800, () => (add(intro[4]!), add(denied), add(card), set({ status: "needs_you" }), emit()));
      return structuredClone(view);
    },
    async agentSessionGet(): Promise<SessionView | null> {
      return view ? structuredClone(view) : null;
    },
    async agentSessionSend(text: string): Promise<void> {
      if (!live()) throw new Error("Sesja agenta już się zakończyła.");
      add({ kind: "user", id: next++, text });
      set({ status: "working" });
      emit();
      later(900, () => (add({ kind: "assistant", id: next++, text: `(podgląd) Rozumiem: „${text}”. W prawdziwej sesji agent odpowiedziałby tutaj.` }), set({ status: "idle" }), emit()));
    },
    async agentSessionAnswer(entry: number, a: SessionAnswer, answers?: Record<string, string>): Promise<void> {
      if (!view) throw new Error("Nie ma sesji agenta.");
      const e = view.entries.find((x) => x.id === entry);
      if (!e || e.kind !== "permission" || e.state !== "pending") throw new Error("Na to pytanie nie można już odpowiedzieć.");
      const state = a === "allow" ? "allowed" : a === "allow_session" ? "allowed_session" : "denied";
      view = {
        ...view,
        status: "working",
        entries: view.entries.map((x) => (x.id === entry && x.kind === "permission" ? { ...x, state, note: answers ? Object.values(answers).join(", ") : "" } : x)),
        remembered: a === "allow_session" ? [...view.remembered, e.tool] : view.remembered,
      };
      emit();
      later(900, () => {
        if (!view) return;
        view = { ...view, entries: view.entries.map((x) => (x.kind === "tool" && x.status === "running" ? { ...x, status: a === "deny" ? "failed" : "done", output: a === "deny" ? "Odmówiono." : "50 wiadomości" } : x)) };
        if (a === "deny") add({ kind: "assistant", id: next++, text: "Bez dostępu do Slacka nie sprawdzę kanału. Mogę za to uporządkować wpisy w Skrzynce." });
        else answer.forEach((x) => add({ ...x, id: next++ }));
        set({ status: "idle", proposal: a === "deny" ? null : { count: 3, at } });
        emit();
      });
    },
    async agentSessionInterrupt(): Promise<void> {
      add({ kind: "notice", id: next++, text: "Przerwano na prośbę użytkownika.", error: false });
      set({ status: "idle" });
      emit();
    },
    async agentSessionStop(): Promise<void> {
      if (!live()) return;
      timers.forEach((t) => globalThis.clearTimeout(t));
      timers = [];
      view = view && { ...view, status: "stopped", ended_at: at, entries: view.entries.map((x) => (x.kind === "permission" && x.state === "pending" ? { ...x, state: "withdrawn" } : x)) };
      emit();
    },
    async agentSessionClose(): Promise<void> {
      if (live()) throw new Error("Sesja jeszcze trwa: najpierw ją zakończ.");
      view = null;
    },
    async onAgentSession(cb: (v: SessionView) => void): Promise<() => void> {
      subs.add(cb);
      return () => void subs.delete(cb);
    },
  };
}

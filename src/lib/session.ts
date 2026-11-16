import { t as tr } from "../i18n";
// The in-app agent session panel, the parts without React: labels, what can
// be done in each state, which buttons a card has, the line for the
// indicator. The backend decides everything that matters (src-tauri
// runs/session); this only presents it.

import type { SessionEntry, SessionStatus, SessionView, SessionStartRequest, ProjectUpdateView, Project } from "../api/types";

export type Card = Extract<SessionEntry, { kind: "permission" }>;

export const STATUS_LABEL: Record<SessionStatus, string> = {
  get starting() { return tr("Uruchamiam…"); },
  get working() { return tr("Agent pracuje"); },
  get needs_you() { return tr("Czeka na twoją zgodę"); },
  get idle() { return tr("Czeka na twoją wiadomość"); },
  get ended() { return tr("Zakończona"); },
  get failed() { return tr("Nie udało się"); },
  get stopped() { return tr("Zakończona przez ciebie"); },
};

export const isLive = (s: SessionStatus) => s === "starting" || s === "working" || s === "needs_you" || s === "idle";

/** A follow-up can be written while the session lives. */
export const canSend = (v: SessionView | null) => !!v && isLive(v.status);
/** "Przerwij" stops the agent's current work (the session stays). */
export const canInterrupt = (v: SessionView | null) => !!v && (v.status === "working" || v.status === "needs_you");

export function pendingCards(v: SessionView | null): Card[] {
  return (v?.entries ?? []).filter((e): e is Card => e.kind === "permission" && e.state === "pending");
}

export type CardAction = { answer: "allow" | "allow_session" | "deny"; label: string; key: string };

/** The buttons of a card: none unless it waits; never any for one the app denied. */
export function cardActions(c: Card): CardAction[] {
  if (c.state !== "pending" || c.questions.length > 0) return [];
  const out: CardAction[] = [{ answer: "allow", label: tr("Zezwól"), key: "z" }];
  if (c.can_session) out.push({ answer: "allow_session", label: tr("Zezwól na tę sesję"), key: "s" });
  out.push({ answer: "deny", label: tr("Odmów"), key: "o" });
  return out;
}

export const CARD_STATE_LABEL: Record<Card["state"], string> = {
  get pending() { return tr("czeka na decyzję"); },
  get allowed() { return tr("zezwolono"); },
  get allowed_session() { return tr("zezwolono na tę sesję"); },
  get denied() { return tr("odmówiono"); },
  get auto_allowed() { return tr("zezwolono (wcześniejsza zgoda na tę sesję)"); },
  get auto_denied() { return tr("odmówiono automatycznie"); },
  get withdrawn() { return tr("wycofane"); },
};

/** The line for the sidebar indicator, or null when there is nothing to show. */
export function indicator(v: SessionView | null, pending = v?.proposal?.count ?? 0): { text: string; attention: boolean } | null {
  if (!v) return null;
  if (v.status === "needs_you") return { text: tr("Agent czeka na zgodę"), attention: true };
  if (v.status === "working" || v.status === "starting") return { text: tr("Agent pracuje…"), attention: false };
  if (pending > 0) return { text: tr("Propozycje gotowe ({0})", pending), attention: true };
  if (v.status === "idle") return { text: tr("Agent czeka na ciebie"), attention: false };
  if (v.status === "failed") return { text: tr("Sesja agenta: błąd"), attention: true };
  return { text: tr("Sesja zakończona"), attention: false };
}

/** "Claude Code" / "Codex". */
export const providerName = (p: SessionView["provider"]) => (p === "claude" ? "Claude Code" : "Codex");

/** A tool name for people: `mcp__slack__read_channel` → `slack · read_channel`. */
export function toolLabel(name: string): string {
  const m = /^mcp__([^_]+(?:_[^_]+)*?)__(.+)$/.exec(name);
  if (m) return `${m[1]} · ${m[2]}`;
  return name.replace("/", " · ");
}

/** Where the proposals wait (a router path). */
export function proposalPath(v: SessionView): { inbox: true } | { project: string } {
  return v.scope.kind === "inbox" ? { inbox: true } : { project: v.scope.project };
}

/** Assistant text for the panel: the `montybrain-zmiany` block is shown as one line (it became the proposals). */
export function displayText(text: string): string {
  return text.replace(/(^|\n)(`{3,}|~{3,})\s*(montybrain|morwisko)-zmiany[^\n]*\n[\s\S]*?(\n\2[ \t]*(?=\n|$)|$)/g, tr("$1[blok zmian: propozycje do zaznaczenia]"));
}


/** Local drafting metadata: focus describes the request, not an access restriction. */
export interface SessionDraft {
  scope: "inbox" | "project";
  project?: string;
  topic?: { slug: string; title: string };
  selection?: string[];
  text: string;
  provider?: SessionStartRequest["provider"];
  origin?: string;
}

export function sessionRequest(draft: SessionDraft, provider: SessionStartRequest["provider"]): SessionStartRequest {
  const focus = draft.scope === "project" && draft.topic
    ? tr("Dotyczy tematu „{0}” ([[{1}/{2}]]). Przeczytaj jego stan, dziennik i zadania w MontyBrain. Zmiany zwróć jako propozycje do akceptacji.\n\n", draft.topic.title, draft.project, draft.topic.slug)
    : "";
  return { scope: draft.scope, project: draft.scope === "project" ? draft.project : undefined, provider, text: focus + draft.text.trim(), selection: draft.scope === "inbox" ? draft.selection : undefined };
}

export function scopeLabel(scope: SessionView["scope"], projects: readonly Pick<Project, "slug" | "name">[]): string {
  return scope.kind === "inbox" ? tr("Skrzynka") : tr("Projekt: {0}", projects.find((p) => p.slug === scope.project)?.name ?? scope.project);
}

export function proposalPending(view: SessionView | null, updates: Record<string, ProjectUpdateView>, loaded: boolean): number {
  if (!view?.proposal) return 0;
  const current = updates[view.scope.kind === "inbox" ? "" : view.scope.project];
  if (!loaded) return view.proposal.count;
  return current?.proposal?.run_id === view.id ? current.pending : 0;
}

/** An answer is sent once, after every question has a nonempty answer. */
export function completeAnswers(card: Card, answers: Record<string, string>): Record<string, string> | null {
  const result: Record<string, string> = {};
  if (!card.questions.length) return null;
  for (const q of card.questions) {
    const answer = answers[q.id]?.trim();
    if (!answer) return null;
    result[q.id] = answer;
  }
  return result;
}

export type ToolEntry = Extract<SessionEntry, { kind: "tool" }>;
export type TimelineEntry = Exclude<SessionEntry, ToolEntry> | {
  kind: "work";
  id: number;
  tools: ToolEntry[];
  status: ToolEntry["status"];
  latest: ToolEntry;
};

/** Group only adjacent tools; messages and permission decisions keep their place. */
export function groupSessionEntries(entries: readonly SessionEntry[]): TimelineEntry[] {
  const result: TimelineEntry[] = [];
  for (const entry of entries) {
    if (entry.kind !== "tool") { result.push(entry); continue; }
    const previous = result.at(-1);
    if (previous?.kind === "work") {
      previous.tools.push(entry);
      previous.status = previous.status === "failed" || entry.status === "failed" ? "failed"
        : previous.status === "running" || entry.status === "running" ? "running" : "done";
      previous.latest = previous.latest.status === "running" ? previous.latest : entry;
    } else {
      result.push({ kind: "work", id: entry.id, tools: [entry], status: entry.status, latest: entry });
    }
  }
  return result;
}

// The in-app agent session panel, the parts without React: labels, what can
// be done in each state, which buttons a card has, the line for the
// indicator. The backend decides everything that matters (src-tauri
// runs/session); this only presents it.

import type { SessionEntry, SessionStatus, SessionView } from "../api/types";

export type Card = Extract<SessionEntry, { kind: "permission" }>;

export const STATUS_LABEL: Record<SessionStatus, string> = {
  starting: "Uruchamiam…",
  working: "Agent pracuje",
  needs_you: "Czeka na twoją zgodę",
  idle: "Czeka na twoją wiadomość",
  ended: "Zakończona",
  failed: "Nie udało się",
  stopped: "Zakończona przez ciebie",
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
  const out: CardAction[] = [{ answer: "allow", label: "Zezwól", key: "z" }];
  if (c.can_session) out.push({ answer: "allow_session", label: "Zezwól na tę sesję", key: "s" });
  out.push({ answer: "deny", label: "Odmów", key: "o" });
  return out;
}

export const CARD_STATE_LABEL: Record<Card["state"], string> = {
  pending: "czeka na decyzję",
  allowed: "zezwolono",
  allowed_session: "zezwolono na tę sesję",
  denied: "odmówiono",
  auto_allowed: "zezwolono (wcześniejsza zgoda na tę sesję)",
  auto_denied: "odmówiono automatycznie",
  withdrawn: "wycofane",
};

/** The line for the sidebar indicator, or null when there is nothing to show. */
export function indicator(v: SessionView | null): { text: string; attention: boolean } | null {
  if (!v) return null;
  if (v.status === "needs_you") return { text: "Agent czeka na zgodę", attention: true };
  if (v.status === "working" || v.status === "starting") return { text: "Agent pracuje…", attention: false };
  if (v.status === "idle") return { text: v.proposal ? "Agent: propozycje gotowe" : "Agent czeka na ciebie", attention: !!v.proposal };
  if (v.status === "failed") return { text: "Sesja agenta: błąd", attention: true };
  return null;
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
  return text.replace(/(^|\n)(`{3,}|~{3,})\s*(montybrain|morwisko)-zmiany[^\n]*\n[\s\S]*?(\n\2[ \t]*(?=\n|$)|$)/g, "$1[blok zmian: propozycje do zaznaczenia]");
}

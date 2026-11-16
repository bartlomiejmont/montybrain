import { t as tr } from "../i18n";
import type { JournalEntry, JournalRefreshMarker, ProviderInfo } from "../api";

/** Lines can move after Markdown edits. Content + multiplicity identifies entries. */
const entryKey = (e: JournalEntry) => JSON.stringify([e.at, e.author, e.text]);
export function userJournal(entries: readonly JournalEntry[]): JournalEntry[] {
  return entries.filter((e) => !["codex", "claude-code"].includes(e.author))
    .sort((a, b) => a.at.localeCompare(b.at) || b.line - a.line);
}

export function newJournalEntries(entries: readonly JournalEntry[], marker?: JournalRefreshMarker): JournalEntry[] {
  const user = userJournal(entries);
  if (!marker) return user.slice(-5);
  const remaining = { ...marker.seen };
  return user.filter((e) => {
    const key = entryKey(e);
    if ((remaining[key] ?? 0) > 0) { remaining[key] = remaining[key]! - 1; return false; }
    return true;
  });
}

/** Capture at start, publish only after a usable result; never read live entries at finish. */
export function journalRefreshMarker(entries: readonly JournalEntry[]): JournalRefreshMarker | undefined {
  const user = userJournal(entries);
  const newest = user.at(-1);
  if (!newest) return undefined;
  const seen: Record<string, number> = {};
  user.forEach((e) => { const key = entryKey(e); seen[key] = (seen[key] ?? 0) + 1; });
  return { at: newest.at, seen };
}

export function journalRefreshLabel(n: number): string {
  if (n === 0) return tr("Brak nowych wpisów");
  if (n === 1) return tr("1 nowy wpis");
  return `${n} ${n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 12 || n % 100 > 14) ? tr("nowe wpisy") : tr("nowych wpisów")}`;
}

export function journalRefreshRequest(entries: readonly JournalEntry[]): string {
  return tr("Odświeżenie tematu · {0}\n\n", journalRefreshLabel(entries.length)) + entries.map((e) => `[${e.at}] ${e.author}\n${e.text}`).join("\n\n");
}

export function journalAgentBlock(loaded: boolean, providers: readonly ProviderInfo[] | null, pending: number, running: boolean): string | null {
  if (!loaded || providers === null) return tr("Sprawdzam dostępność agenta i propozycje projektu…");
  if (pending > 0) return tr("Projekt ma nierozstrzygnięte propozycje. Najpierw przejrzyj zmiany.");
  if (!providers.some((p) => p.available)) return tr("Żaden agent nie jest dostępny. Odświeżenie wymaga dostępnego agenta.");
  if (running) return tr("W tle trwa już analiza. Poczekaj na jej zakończenie lub ją przerwij.");
  return null;
}

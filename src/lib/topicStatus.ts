import { t as tr } from "../i18n";
// Quick topic status changes in the project view: the status pill's menu and
// the selection mode ("Zaznacz") that sets one status on many topics at once.
// Pure logic; the view is ProjectView + TopicItem.

import type { TopicStatus } from "../api/types";
import { TOPIC_STATUSES, TOPIC_STATUS_LABEL } from "./labels";
import { plural } from "./text";

/** The topic row's counter in words: "1 otwarte zadanie z 1", "2 otwarte zadania z 3", "5 otwartych zadań z 8". */
export function openCount(open: number, total: number): string {
  return tr("{0} {1} z {2}", open, plural(open, tr("otwarte zadanie"), tr("otwarte zadania"), tr("otwartych zadań")), total);
}

/** Accessible name of the status pill: "Status tematu Płatności: W toku. Zmień". */
export function statusPillLabel(title: string, status: TopicStatus): string {
  return tr("Status tematu {0}: {1}. Zmień", title, TOPIC_STATUS_LABEL[status]);
}

/** The five statuses for the menu; `current` checked (none in the bulk menu). */
export function statusChoices(current: TopicStatus | null): { status: TopicStatus; label: string; checked: boolean }[] {
  return TOPIC_STATUSES.map((status) => ({ status, label: TOPIC_STATUS_LABEL[status], checked: status === current }));
}

export interface Selection {
  selected: ReadonlySet<string>;
  /** The last plainly clicked row: one end of a Shift-click range. */
  anchor: string | null;
}

export const EMPTY_SELECTION: Selection = { selected: new Set(), anchor: null };

/**
 * A click on the checkbox of `target`. A plain click toggles it and makes it
 * the anchor. A Shift-click sets every row between the anchor and `target`
 * (in `order`, the rows as shown) to the anchor's state; without a visible
 * anchor it is a plain click.
 */
export function clickSelect(order: readonly string[], sel: Selection, target: string, shift: boolean): Selection {
  const a = sel.anchor === null ? -1 : order.indexOf(sel.anchor);
  const t = order.indexOf(target);
  if (t === -1) return sel;
  const next = new Set(sel.selected);
  if (!shift || a === -1) {
    if (next.has(target)) next.delete(target);
    else next.add(target);
    return { selected: next, anchor: target };
  }
  const on = sel.selected.has(sel.anchor!);
  for (let i = Math.min(a, t); i <= Math.max(a, t); i++) {
    if (on) next.add(order[i]!);
    else next.delete(order[i]!);
  }
  return { selected: next, anchor: sel.anchor };
}

/** Keeps only rows still shown (a topic deleted, the done group closed). */
export function pruneSelection(sel: Selection, order: readonly string[]): Selection {
  const shown = new Set(order);
  const selected = new Set([...sel.selected].filter((k) => shown.has(k)));
  const anchor = sel.anchor !== null && shown.has(sel.anchor) ? sel.anchor : null;
  return selected.size === sel.selected.size && anchor === sel.anchor ? sel : { selected, anchor };
}

/** "Zaznaczono 3". */
export function selectionLabel(n: number): string {
  return tr("Zaznaczono {0}", n);
}

/** The selected topics that a bulk change to `status` would change (the others already have it). */
export function bulkTargets<T extends { slug: string; status: TopicStatus }>(topics: readonly T[], selected: ReadonlySet<string>, status: TopicStatus): T[] {
  return topics.filter((t) => selected.has(t.slug) && t.status !== status);
}

export interface BulkResult {
  title: string;
  /** `null` when saved, else why not. */
  error: string | null;
}

/**
 * The one toast after a bulk change: "Zmieniono status 3 tematów na „W toku”.",
 * failures named: "Zmieniono status 2 z 3 tematów na „W toku”. Nie udało się: „Kasa” (…)."
 */
export function bulkSummary(results: readonly BulkResult[], status: TopicStatus): string {
  const to = `„${TOPIC_STATUS_LABEL[status]}”`;
  if (results.length === 0) return tr("Zaznaczone tematy mają już status {0}.", to);
  const failed = results.filter((r) => r.error !== null);
  const ok = results.length - failed.length;
  const topics = (n: number) => plural(n, tr("tematu"), tr("tematów"), tr("tematów"));
  if (failed.length === 0) return tr("Zmieniono status {0} {1} na {2}.", ok, topics(ok), to);
  const named = failed
    .slice(0, 3)
    .map((r) => `„${r.title}” (${r.error})`)
    .join(", ");
  const more = failed.length > 3 ? tr(" i {0} więcej", failed.length - 3) : "";
  if (ok === 0) return tr("Nie udało się zmienić statusu: {0}{1}.", named, more);
  return tr("Zmieniono status {0} z {1} {2} na {3}. Nie udało się: {4}{5}.", ok, results.length, topics(results.length), to, named, more);
}

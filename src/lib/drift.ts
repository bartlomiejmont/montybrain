// Drift between repositories and topics, as the project view shows it: the
// one-line summary, the prepared request for "Zapytaj agenta", and the
// "Podobny do" hint on a proposed new topic. Pure.

import type { BranchRef, ProjectDrift, ProposedChange, Topic } from "../api/types";
import { fold, plural } from "./text";

const branches = (n: number) => `${n} ${plural(n, "gałąź", "gałęzie", "gałęzi")}`;

/** "Rozjazd: 3 gałęzie bez tematu · „Płatności”: 4 gałęzie w 2 repozytoriach · 12 nowych commitów od sprawdzenia"; `null` when there is nothing to say. */
export function driftLine(d: ProjectDrift | null | undefined, titleOf: (slug: string) => string): string | null {
  if (!d) return null;
  const parts: string[] = [];
  if (d.unknown.length) parts.push(`${branches(d.unknown.length)} bez tematu`);
  for (const s of d.spread) parts.push(`„${titleOf(s.topic)}”: ${branches(s.branches)}${s.repos > 1 ? ` w ${s.repos} repozytoriach` : ""}`);
  if (d.moved.length) parts.push(`${d.moved.length} ${plural(d.moved.length, "temat", "tematy", "tematów")} bez wpisu mimo commitów`);
  if (d.new_commits) parts.push(`${d.new_commits} ${plural(d.new_commits, "nowy commit", "nowe commity", "nowych commitów")} od sprawdzenia`);
  return parts.length ? `Rozjazd: ${parts.join(" · ")}` : null;
}

/** The instruction "Zapytaj agenta" starts the request recipe with (the only paid path of the drift detector). */
export function driftRequest(d: ProjectDrift, titleOf: (slug: string) => string): string {
  const lines: string[] = [];
  if (d.unknown.length) {
    lines.push("Gałęzie, które nie pasują do żadnego tematu:");
    for (const b of d.unknown) lines.push(`- ${b.repo}: ${b.branch} (ostatni commit ${b.last_commit}${b.worktree ? `, drzewo robocze ${b.worktree}` : ""})`);
  }
  for (const s of d.spread) lines.push(`Temat „${titleOf(s.topic)}” ma ${branches(s.branches)}${s.repos > 1 ? ` w ${s.repos} repozytoriach` : ""}.`);
  for (const m of d.moved) lines.push(`Temat „${titleOf(m.topic)}”: gałąź ${m.repo}/${m.branch} ma commity z ${m.last_commit}, a ostatni wpis w dzienniku jest ${m.last_entry ? `z ${m.last_entry}` : "brak"}.`);
  lines.push(
    "Sprawdź w repozytoriach, czym są te gałęzie, i zaproponuj: dopisanie ich do istniejących tematów (topic_log z polem branches) albo nowe tematy (new_topic z polem branches). Porzucone lub drobne gałęzie tylko wymień w raporcie.",
  );
  return lines.join("\n");
}

const NOISE = new Set(["dla", "oraz", "przez", "jako", "nowy", "nowa", "nowe", "temat", "projekt", "and", "the", "for"]);

/** Significant folded words of a title. */
export function titleWords(title: string): string[] {
  return [...new Set(fold(title).split(/[^a-z0-9]+/).filter((w) => w.length >= 3 && !NOISE.has(w)))];
}

function stem(w: string): string {
  return w.length <= 4 ? w : w.slice(0, Math.min(8, Math.max(4, w.length - 2)));
}

function sameWord(a: string, b: string): boolean {
  if (a === b) return true;
  const fits = (w: string, st: string) => st.length >= 4 && w.startsWith(st) && (st.length >= 5 || w.length <= st.length + 2);
  return fits(b, stem(a)) || fits(a, stem(b));
}

/**
 * An existing topic a proposed new topic resembles: shared word stems in the
 * titles (the most, at least one, no tie), or a branch of the proposal that is
 * already linked to that topic.
 */
export function similarTopic(change: ProposedChange, topics: readonly Topic[], links: readonly { repo: string; branch: string; topic: string }[] = []): Topic | null {
  if (change.kind !== "new_topic") return null;
  const open = topics.filter((t) => t.status !== "done");
  const byLink = (change.branches ?? []).map((b: BranchRef) => links.find((l) => l.repo === b.repo && l.branch === b.branch)?.topic).find(Boolean);
  if (byLink) return open.find((t) => t.slug === byLink) ?? null;
  const words = titleWords(change.title);
  let best: Topic | null = null;
  let bestScore = 0;
  let tie = false;
  for (const t of open) {
    const tw = titleWords(`${t.title} ${t.slug}`);
    const score = words.filter((w) => tw.some((x) => sameWord(w, x))).length;
    if (score === 0) continue;
    if (score > bestScore) {
      best = t;
      bestScore = score;
      tie = false;
    } else if (score === bestScore) tie = true;
  }
  return tie ? null : best;
}

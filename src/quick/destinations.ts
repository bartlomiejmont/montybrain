import { getLocale, t as tr } from "../i18n";
// Where the quick-capture window can put a task: the inbox, every
// non-archived project (its projekt.md) and the project's open topics.

import type { Snapshot, TaskLocation } from "../api/types";
import { fold } from "../lib/text";

export interface Destination {
  key: string;
  /** Text in the list ("Skrzynka", project name, topic title). */
  label: string;
  /** Text on the picker button ("WhenUBuy / A/B testy"). */
  full: string;
  /** 0 = inbox / project, 1 = topic (indented under its project). */
  depth: 0 | 1;
  color: string | null;
  location: TaskLocation;
  /** Lower-cased, diacritics-free text used for filtering. */
  haystack: string;
}

export function destinationKey(loc: TaskLocation): string {
  if (loc.kind === "inbox") return "inbox";
  if (loc.kind === "project") return `p:${loc.project}`;
  return `t:${loc.project}/${loc.topic}`;
}

const byText = (a: string, b: string) => a.localeCompare(b, getLocale(), { sensitivity: "base" });

export function destinations(snap: Snapshot): Destination[] {
  const inbox: TaskLocation = { kind: "inbox" };
  const out: Destination[] = [
    { key: "inbox", label: tr("Skrzynka"), full: tr("Skrzynka"), depth: 0, color: null, location: inbox, haystack: "skrzynka inbox" },
  ];
  const projects = snap.projects.filter((p) => p.state !== "archived").sort((a, b) => byText(a.name, b.name));
  for (const p of projects) {
    const loc: TaskLocation = { kind: "project", project: p.slug };
    out.push({
      key: destinationKey(loc),
      label: p.name,
      full: p.name,
      depth: 0,
      color: p.color,
      location: loc,
      haystack: fold(`${p.name} ${p.slug}`),
    });
    const topics = snap.topics
      .filter((t) => t.project === p.slug && t.status !== "done")
      .sort((a, b) => byText(a.title, b.title));
    for (const t of topics) {
      const tl: TaskLocation = { kind: "topic", project: p.slug, topic: t.slug };
      out.push({
        key: destinationKey(tl),
        label: t.title,
        full: `${p.name} / ${t.title}`,
        depth: 1,
        color: p.color,
        location: tl,
        haystack: fold(`${p.name} ${p.slug} ${t.title} ${t.slug}`),
      });
    }
  }
  return out;
}

/** Every whitespace-separated term must match (case/diacritics ignored). */
export function filterDestinations(list: Destination[], query: string): Destination[] {
  const terms = fold(query).split(/\s+/).filter(Boolean);
  if (terms.length === 0) return list;
  return list.filter((d) => terms.every((t) => d.haystack.includes(t)));
}

/** The remembered destination if it still exists, else the inbox. */
export function resolveDestination(list: Destination[], saved: TaskLocation | null | undefined): Destination {
  const key = saved ? destinationKey(saved) : "inbox";
  return list.find((d) => d.key === key) ?? list[0]!;
}

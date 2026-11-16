// References between topics and tasks, written in Markdown like Obsidian
// wikilinks (docs/format.md): `[[slug]]` (a topic of the same project),
// `[[projekt/slug]]` (any project), `^id` (a task). A standalone line
// `Czeka na: <reference>` is the directed "waits for" kind; any other
// reference is "powiązane". Read only from topic summaries and task notes,
// fenced code skipped; the journal is never read. Pure.

import type { Snapshot, Task, Topic } from "../api/types";

export type RefTarget = { kind: "topic"; project: string; slug: string } | { kind: "task"; id: string };

export interface Ref {
  target: RefTarget;
  /** `Czeka na: …` on its own line. */
  waits: boolean;
  /** The reference is the whole line (`Powiązane: [[…]]` / `Czeka na: [[…]]`): removable from the map. */
  standalone: boolean;
  /** The line it is on (0-based, in the source text). */
  line: number;
}

export const WAITS_PREFIX = "Czeka na:";
export const RELATED_PREFIX = "Powiązane:";

const ID = /^[a-z0-9]{6}$/;
const SLUG = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

/** Every reference in `text` (fenced code skipped), `project` being the project the text belongs to. */
export function parseRefs(text: string, project: string | null): Ref[] {
  const out: Ref[] = [];
  let fence: string | null = null;
  const lines = text.split("\n");
  lines.forEach((raw, line) => {
    const t = raw.trim();
    const f = /^(`{3,}|~{3,})/.exec(t);
    if (fence) {
      if (f && f[1]![0] === fence[0] && f[1]!.length >= fence.length) fence = null;
      return;
    }
    if (f) {
      fence = f[1]!;
      return;
    }
    // Inline code spans do not count either.
    const plain = raw.replace(/`[^`]*`/g, (m) => " ".repeat(m.length));
    const waitsLine = plain.trim().startsWith(WAITS_PREFIX);
    const found: { target: RefTarget; start: number; end: number }[] = [];
    for (const m of plain.matchAll(/\[\[([^\]\n|]+)(?:\|[^\]\n]*)?\]\]/g)) {
      const body = m[1]!.trim();
      const slash = body.indexOf("/");
      const p = slash >= 0 ? body.slice(0, slash).trim() : project;
      const slug = slash >= 0 ? body.slice(slash + 1).trim() : body;
      if (!p || !SLUG.test(slug) || !SLUG.test(p)) continue;
      found.push({ target: { kind: "topic", project: p, slug }, start: m.index!, end: m.index! + m[0].length });
    }
    for (const m of plain.matchAll(/(^|[\s(,;:])\^([a-z0-9]{6})(?![a-z0-9])/g)) {
      const id = m[2]!;
      if (!ID.test(id)) continue;
      const start = m.index! + m[1]!.length;
      found.push({ target: { kind: "task", id }, start, end: start + 1 + id.length });
    }
    found.sort((a, b) => a.start - b.start);
    const rest = (prefix: string) => {
      const body = plain.trim().slice(prefix.length).trim();
      return found.length === 1 && body === plain.slice(found[0]!.start, found[0]!.end).trim();
    };
    const standalone = rest(WAITS_PREFIX) || rest(RELATED_PREFIX);
    for (const f2 of found) out.push({ target: f2.target, waits: waitsLine, standalone: standalone && found.length === 1, line });
  });
  return out;
}

/** Prose for display beside a dedicated relations section; Markdown on disk is unchanged. */
export function referenceProse(text: string, project: string | null): string {
  const relationLines = new Set(parseRefs(text, project).filter((r) => r.standalone).map((r) => r.line));
  return text.split("\n").filter((_, i) => !relationLines.has(i)).join("\n").trim();
}

// --- the graph -----------------------------------------------------------------------

export type NodeKey = string;

export const topicKey = (project: string, slug: string): NodeKey => `t:${project}/${slug}`;

/** A task's key: its id, or (before normalization) a fallback that cannot be a target. */
export function taskKey(t: Task): NodeKey {
  if (t.id) return `k:${t.id}`;
  const l = t.location;
  const where = l.kind === "inbox" ? "inbox" : l.kind === "project" ? l.project : `${l.project}/${l.topic}`;
  return `k?:${where}#${t.line}`;
}

export type EdgeKind = "related" | "waits";

export interface Edge {
  from: NodeKey;
  to: NodeKey;
  kind: EdgeKind;
  /** Written as a whole line in the source (`Powiązane:` / `Czeka na:`): removable. */
  standalone: boolean;
  /** The text the reference was found in. */
  source: "summary" | "notes";
}

export interface Dangling {
  from: NodeKey;
  /** As written, e.g. "[[whenubuy/nie-ma]]" or "^zz91kq". */
  text: string;
  waits: boolean;
}

export interface Graph {
  edges: Edge[];
  dangling: Dangling[];
}

export function sameTarget(a: RefTarget, b: RefTarget): boolean {
  if (a.kind === "task" || b.kind === "task") return a.kind === b.kind && (a as { id: string }).id === (b as { id: string }).id;
  return a.project === b.project && a.slug === b.slug;
}

export function refText(t: RefTarget, from: string | null): string {
  if (t.kind === "task") return `^${t.id}`;
  return t.project === from ? `[[${t.slug}]]` : `[[${t.project}/${t.slug}]]`;
}

/** Edges from topic summaries and task notes (journal never read). Self references are dropped; duplicates once. */
export function buildGraph(snap: Pick<Snapshot, "topics" | "tasks">): Graph {
  const topics = new Set(snap.topics.map((t) => topicKey(t.project, t.slug)));
  const tasks = new Set(snap.tasks.filter((t) => t.id).map((t) => `k:${t.id}`));
  const edges: Edge[] = [];
  const dangling: Dangling[] = [];
  const seen = new Set<string>();
  const add = (from: NodeKey, project: string | null, text: string, source: Edge["source"]) => {
    for (const r of parseRefs(text, project)) {
      const to = r.target.kind === "topic" ? topicKey(r.target.project, r.target.slug) : `k:${r.target.id}`;
      const exists = r.target.kind === "topic" ? topics.has(to) : tasks.has(to);
      if (!exists) {
        dangling.push({ from, text: refText(r.target, project), waits: r.waits });
        continue;
      }
      if (to === from) continue;
      const kind: EdgeKind = r.waits ? "waits" : "related";
      const id = `${from}>${to}>${kind}`;
      if (seen.has(id)) {
        if (r.standalone) {
          const e = edges.find((x) => `${x.from}>${x.to}>${x.kind}` === id);
          if (e) e.standalone = true;
        }
        continue;
      }
      seen.add(id);
      edges.push({ from, to, kind, standalone: r.standalone, source });
    }
  };
  for (const t of snap.topics) add(topicKey(t.project, t.slug), t.project, t.summary, "summary");
  for (const t of snap.tasks) if (t.notes) add(taskKey(t), t.location.kind === "inbox" ? null : t.location.project, t.notes, "notes");
  return { edges, dangling };
}

/** Edges touching `key`, each with the other end and the direction. */
export function neighbours(g: Graph, key: NodeKey): { edge: Edge; other: NodeKey; outgoing: boolean }[] {
  const out: { edge: Edge; other: NodeKey; outgoing: boolean }[] = [];
  for (const e of g.edges) {
    if (e.from === key) out.push({ edge: e, other: e.to, outgoing: true });
    else if (e.to === key) out.push({ edge: e, other: e.from, outgoing: false });
  }
  // What it waits for, what waits for it, then the related ones; stable within each.
  const rank = (n: { edge: Edge; outgoing: boolean }) => (n.edge.kind === "waits" ? (n.outgoing ? 0 : 1) : 2);
  return out.sort((a, b) => rank(a) - rank(b) || a.other.localeCompare(b.other));
}

/**
 * What `key` waits for, link by link, up to `depth` levels: at each step the
 * first "waits" target (in key order); a cycle is cut where it closes.
 */
export function waitChain(g: Graph, key: NodeKey, depth = 4): { chain: NodeKey[]; cycle: boolean } {
  const chain: NodeKey[] = [];
  const seen = new Set<NodeKey>([key]);
  let at = key;
  for (let i = 0; i < depth; i++) {
    const next = g.edges
      .filter((e) => e.from === at && e.kind === "waits")
      .map((e) => e.to)
      .sort()[0];
    if (!next) return { chain, cycle: false };
    if (seen.has(next)) return { chain, cycle: true };
    chain.push(next);
    seen.add(next);
    at = next;
  }
  return { chain, cycle: false };
}

/** Per node: number of connections and whether it waits for something. */
export function counts(g: Graph): Map<NodeKey, { links: number; waits: boolean }> {
  const m = new Map<NodeKey, { links: number; waits: boolean }>();
  const get = (k: NodeKey) => {
    let v = m.get(k);
    if (!v) m.set(k, (v = { links: 0, waits: false }));
    return v;
  };
  for (const e of g.edges) {
    get(e.from).links++;
    get(e.to).links++;
    if (e.kind === "waits") get(e.from).waits = true;
  }
  return m;
}

// --- editing the standalone lines (stage 2: "Połącz z…") ----------------------------------

/** `summary` with one more standalone line (`Powiązane: [[p/s]]` or `Czeka na: [[p/s]]`), the rest unchanged. */
export function appendRefLine(summary: string, kind: EdgeKind, target: Topic, from: string): string {
  // Always the project-qualified form: it stays right if the text is ever moved.
  void from;
  const line = `${kind === "waits" ? WAITS_PREFIX : RELATED_PREFIX} [[${target.project}/${target.slug}]]`;
  if (!summary.trim()) return line;
  return summary.endsWith("\n") ? `${summary}${line}` : `${summary}\n${line}`;
}

/** `summary` without the standalone line that makes the edge to `target` (exactly that line), or null when there is none. */
export function removeRefLine(summary: string, kind: EdgeKind, target: RefTarget, from: string): string | null {
  const lines = summary.split("\n");
  // Lines inside fenced code never count: parse the whole text, then match by line.
  const hit = parseRefs(summary, from).find((r) => r.standalone && r.waits === (kind === "waits") && sameTarget(r.target, target));
  const i = hit ? hit.line : -1;
  if (i < 0) return null;
  lines.splice(i, 1);
  return lines.join("\n");
}

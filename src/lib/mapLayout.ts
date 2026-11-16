// The layout of „Mapa": a pure function from what is shown to rectangles.
//
// * Project islands in two columns, in sidebar order (odd ones left, even
//   ones right), each column packed top to bottom with the normal gutter.
// * Inside an island: the project header, then the topics as rows along one
//   vertical spine, ordered by creation date, then slug. Nothing else (status,
//   updates, task counts, the journal) influences order or position.
// * Accordion: one topic per island can be expanded (up to eight open tasks,
//   then "+N"). Its block pushes down the rows below it in its island and the
//   islands below it in the same column, by exactly its height; collapsing
//   restores them. The other column never moves, nor anything above the
//   expanded row (so the clicked row stays where it is on screen).
// * A new topic appends at the end of its island and pushes only the islands
//   below it in the same column, by one row. Data changes the layout does not
//   read (tasks of collapsed topics, the journal, statuses) move nothing.
// * Zoom only scales (the view does that); nothing here depends on it.

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface LayoutTopic {
  slug: string;
  /** Creation date (or null: sorted last, then by slug). */
  created: string | null;
}

export interface LayoutProject {
  slug: string;
  topics: LayoutTopic[];
}

export interface LayoutInput {
  /** Visible projects, in sidebar order. */
  projects: LayoutProject[];
  /** The expanded topic per project (at most one). */
  expanded: Record<string, string | undefined>;
  /** Open tasks (keys, in list order) of a topic; only asked for expanded topics. */
  openTasks: (project: string, topic: string) => string[];
}

export type RowKind = "project" | "topic" | "task" | "more";

export interface LayoutRow {
  /** `p:slug`, `t:project/slug`, a task key, or `more:project/slug`. */
  key: string;
  kind: RowKind;
  project: string;
  topic?: string;
  rect: Rect;
}

export interface IslandLayout {
  project: string;
  /** The frame (hugs the content). */
  rect: Rect;
  /** Same as `rect` (kept for callers that ask for the island's space). */
  slot: Rect;
  /** The spine: from below the header to the middle of the last topic row. */
  spine: { x: number; y1: number; y2: number };
}

export interface MapLayout {
  islands: IslandLayout[];
  /** In document order: per island the header, its topics, the expanded tasks. */
  rows: LayoutRow[];
  width: number;
  height: number;
}

export const ISLAND_W = 460;
export const COL_GAP = 120;
export const ROW_GAP = 48;
export const MARGIN = 48;
export const HEADER_H = 44;
export const TOPIC_H = 34;
export const TASK_H = 24;
export const PAD = 12;
export const MAX_TASKS = 8;
const INDENT = 28;
const TASK_INDENT = 56;

export function sortTopics<T extends LayoutTopic>(topics: readonly T[]): T[] {
  return [...topics].sort((a, b) => {
    const ca = a.created ?? "￿";
    const cb = b.created ?? "￿";
    return ca < cb ? -1 : ca > cb ? 1 : a.slug < b.slug ? -1 : a.slug > b.slug ? 1 : 0;
  });
}

export function layoutMap(input: LayoutInput): MapLayout {
  const rows: LayoutRow[] = [];
  const islands: IslandLayout[] = [];
  // Where the next island of each column starts.
  const colTop = [MARGIN, MARGIN];
  const width = MARGIN * 2 + ISLAND_W * 2 + COL_GAP;

  input.projects.forEach((p, i) => {
    const col = i % 2;
    const x = MARGIN + col * (ISLAND_W + COL_GAP);
    const top = colTop[col]!;
    rows.push({ key: `p:${p.slug}`, kind: "project", project: p.slug, rect: { x: x + PAD, y: top + 6, w: ISLAND_W - 2 * PAD, h: HEADER_H - 12 } });
    let cy = top + HEADER_H;
    const topics = sortTopics(p.topics);
    let lastMid = cy;
    for (const t of topics) {
      rows.push({ key: `t:${p.slug}/${t.slug}`, kind: "topic", project: p.slug, topic: t.slug, rect: { x: x + INDENT, y: cy, w: ISLAND_W - INDENT - PAD, h: TOPIC_H - 4 } });
      lastMid = cy + (TOPIC_H - 4) / 2;
      cy += TOPIC_H;
      if (input.expanded[p.slug] === t.slug) {
        const open = input.openTasks(p.slug, t.slug);
        for (const k of open.slice(0, MAX_TASKS)) {
          rows.push({ key: k, kind: "task", project: p.slug, topic: t.slug, rect: { x: x + TASK_INDENT, y: cy, w: ISLAND_W - TASK_INDENT - PAD, h: TASK_H - 4 } });
          cy += TASK_H;
        }
        if (open.length > MAX_TASKS) {
          rows.push({ key: `more:${p.slug}/${t.slug}`, kind: "more", project: p.slug, topic: t.slug, rect: { x: x + TASK_INDENT, y: cy, w: 120, h: TASK_H - 4 } });
          cy += TASK_H;
        }
        cy += 4;
      }
    }
    const frameH = cy - top + PAD;
    colTop[col] = top + frameH + ROW_GAP;
    islands.push({
      project: p.slug,
      rect: { x, y: top, w: ISLAND_W, h: frameH },
      slot: { x, y: top, w: ISLAND_W, h: frameH },
      spine: { x: x + INDENT - 12, y1: top + HEADER_H - 4, y2: topics.length ? lastMid : top + HEADER_H - 4 },
    });
  });
  const height = input.projects.length ? Math.max(...colTop) - ROW_GAP + MARGIN : 2 * MARGIN;
  return { islands, rows, width, height };
}

/** The view (translate + scale) that fits the whole layout into a box. */
export function fitView(l: { width: number; height: number }, box: { w: number; h: number }): { x: number; y: number; z: number } {
  const z = Math.min(1.25, Math.max(0.2, Math.min(box.w / l.width, box.h / l.height)));
  return { z, x: (box.w - l.width * z) / 2, y: Math.max(0, (box.h - l.height * z) / 2) };
}

export function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/**
 * A curve from row `a` to row `b` through the gutters: out of the island's side
 * facing the gutter, along the gutter, into the other island; never across rows.
 * Both columns use the middle gutter between columns so edges stay within the
 * layout bounds and never cross the spine or row text.
 */
export function routeCurve(a: Rect, b: Rect, aIsland: Rect, bIsland: Rect, lane: number): string {
  const mid = (aIsland.x < bIsland.x ? aIsland.x + aIsland.w : bIsland.x + bIsland.w) + COL_GAP / 2;
  const sameCol = aIsland.x === bIsland.x;
  const leftCol = aIsland.x === MARGIN;
  const gx = sameCol ? (leftCol ? aIsland.x + aIsland.w + 16 + lane * 8 : aIsland.x - 16 - lane * 8) : mid + (lane - 3.5) * 8;
  const ay = a.y + a.h / 2;
  const by = b.y + b.h / 2;
  const ax = aIsland.x + aIsland.w <= gx ? aIsland.x + aIsland.w : aIsland.x;
  const bx = bIsland.x + bIsland.w <= gx ? bIsland.x + bIsland.w : bIsland.x;
  const r = Math.min(10, Math.abs(by - ay) / 2);
  const dir = by >= ay ? 1 : -1;
  const sa = gx >= ax ? 1 : -1;
  const sb = gx >= bx ? 1 : -1;
  return [
    `M ${ax} ${ay}`,
    `H ${gx - sa * r}`,
    `Q ${gx} ${ay} ${gx} ${ay + dir * r}`,
    `V ${by - dir * r}`,
    `Q ${gx} ${by} ${gx - sb * r} ${by}`,
    `H ${bx}`,
  ].join(" ");
}

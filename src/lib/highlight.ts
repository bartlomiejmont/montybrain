// Highlighting for search results. The backend matches case- and
// diacritics-insensitively (`fold`), so we match on the folded text and map the
// hits back to the original string.

import { fold } from "./text";

export interface Segment {
  text: string;
  hit: boolean;
}

export function searchTerms(query: string): string[] {
  return fold(query).split(/\s+/).filter(Boolean);
}

/** [start, end) ranges in `text` (UTF-16 offsets) matching any term, merged. */
export function matchRanges(text: string, terms: string[]): [number, number][] {
  if (terms.length === 0 || !text) return [];
  let folded = "";
  const start: number[] = [];
  const end: number[] = [];
  let offset = 0;
  for (const ch of text) {
    const next = offset + ch.length;
    const f = fold(ch);
    if (f === "") {
      // A stray combining mark: belongs to the previous character.
      if (end.length) end[end.length - 1] = next;
    } else {
      for (let i = 0; i < f.length; i++) {
        start.push(offset);
        end.push(next);
      }
      folded += f;
    }
    offset = next;
  }
  const ranges: [number, number][] = [];
  for (const term of terms) {
    let from = 0;
    for (;;) {
      const i = folded.indexOf(term, from);
      if (i < 0) break;
      ranges.push([start[i]!, end[i + term.length - 1]!]);
      from = i + term.length;
    }
  }
  ranges.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const merged: [number, number][] = [];
  for (const r of ranges) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([r[0], r[1]]);
  }
  return merged;
}

export function highlight(text: string, terms: string[]): Segment[] {
  const out: Segment[] = [];
  let at = 0;
  for (const [s, e] of matchRanges(text, terms)) {
    if (s > at) out.push({ text: text.slice(at, s), hit: false });
    out.push({ text: text.slice(s, e), hit: true });
    at = e;
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false });
  return out.length ? out : [{ text, hit: false }];
}

/**
 * A short fragment of `text` around the first match (whitespace collapsed),
 * highlighted; null when nothing matches.
 */
export function fragment(text: string, terms: string[], radius = 48): Segment[] | null {
  const flat = text.replace(/\s+/g, " ").trim();
  const ranges = matchRanges(flat, terms);
  if (ranges.length === 0) return null;
  const [s, e] = ranges[0]!;
  let from = Math.max(0, s - radius);
  let to = Math.min(flat.length, e + radius);
  if (from > 0) {
    const sp = flat.indexOf(" ", from);
    if (sp >= 0 && sp < s) from = sp + 1;
  }
  if (to < flat.length) {
    const sp = flat.lastIndexOf(" ", to);
    if (sp > e) to = sp;
  }
  const slice = flat.slice(from, to);
  const segs = highlight(slice, terms);
  if (from > 0) segs.unshift({ text: "…", hit: false });
  if (to < flat.length) segs.push({ text: "…", hit: false });
  return segs;
}

// Text helpers that mirror montybrain-core (`fold`, `slugify`) closely enough for
// the browser mock and client-side matching.

const EXTRA: Record<string, string> = { ł: "l", Ł: "l", ß: "ss", æ: "ae", Æ: "ae", œ: "oe", Œ: "oe", ø: "o", Ø: "o" };

/** Lowercase + strip diacritics (ą→a, ł→l, …). */
export function fold(s: string): string {
  return s
    .replace(/[łŁßæÆœŒøØ]/g, (c) => EXTRA[c] ?? c)
    .normalize("NFD")
    .replace(/\p{M}+/gu, "")
    .toLowerCase();
}

/** Kebab-case ASCII slug, max 60 chars, `bez-nazwy` when empty (as in core). */
export function slugify(name: string): string {
  let s = fold(name)
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (s.length > 60) {
    const cut = s.slice(0, 60);
    const dash = cut.lastIndexOf("-");
    s = (dash > 0 ? cut.slice(0, dash) : cut).replace(/-+$/, "");
  }
  return s || "bez-nazwy";
}

/** `base`, or `base-2`, `base-3`, … – the first one not taken. */
export function uniqueSlug(base: string, taken: (s: string) => boolean): string {
  if (!taken(base)) return base;
  for (let n = 2; ; n++) {
    const s = `${base}-${n}`;
    if (!taken(s)) return s;
  }
}

/** Collapse whitespace to single spaces (titles are one line in the files). */
export function oneLine(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

/** Short label for a URL chip: host + a hint of the path. */
export function linkLabel(url: string): string {
  try {
    const u = new URL(url);
    if (u.protocol === "mailto:") return u.pathname;
    const host = u.hostname.replace(/^www\./, "");
    const mr = u.pathname.match(/\/(merge_requests|pull|issues)\/(\d+)/);
    if (mr) return `${host} ${mr[1] === "merge_requests" ? "!" : "#"}${mr[2]}`;
    const last = u.pathname.split("/").filter(Boolean).pop();
    return last && last.length <= 24 ? `${host}/…/${last}` : host;
  } catch {
    return url.length > 32 ? `${url.slice(0, 31)}…` : url;
  }
}

/** Polish plural: plural(5, "zadanie", "zadania", "zadań"). */
export function plural(n: number, one: string, few: string, many: string): string {
  if (n === 1) return one;
  const d = n % 10;
  const t = n % 100;
  if (d >= 2 && d <= 4 && (t < 12 || t > 14)) return few;
  return many;
}

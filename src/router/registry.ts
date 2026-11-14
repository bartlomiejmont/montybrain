// View registry. A view is a path pattern + a component; new views (kanban,
// map, …) register themselves in src/views/index.ts without touching others.

import type { ComponentType } from "react";

export type Params = Record<string, string>;

export interface Route {
  /** Matched view id, or null when nothing matched. */
  view: string | null;
  path: string;
  params: Params;
  query: URLSearchParams;
}

export interface ViewProps {
  route: Route;
}

export interface ViewDef {
  id: string;
  /** Hash path pattern without leading slash, e.g. "p/:project". */
  pattern: string;
  component: ComponentType<ViewProps>;
}

const views: ViewDef[] = [];

export function registerView(def: ViewDef): void {
  if (views.some((v) => v.id === def.id)) throw new Error(`view "${def.id}" registered twice`);
  views.push(def);
}

export function registeredViews(): readonly ViewDef[] {
  return views;
}

export function getView(id: string | null): ViewDef | undefined {
  return views.find((v) => v.id === id);
}

/** Matches "p/:project" against "p/whenubuy" → { project: "whenubuy" }. */
export function matchPattern(pattern: string, path: string): Params | null {
  const ps = pattern.split("/").filter(Boolean);
  const xs = path.split("/").filter(Boolean);
  if (ps.length !== xs.length) return null;
  const params: Params = {};
  for (let i = 0; i < ps.length; i++) {
    const p = ps[i]!;
    const x = xs[i]!;
    if (p.startsWith(":")) {
      try {
        params[p.slice(1)] = decodeURIComponent(x);
      } catch {
        return null;
      }
    } else if (p !== x) {
      return null;
    }
  }
  return params;
}

/** Parses a location hash ("#/p/whenubuy?topic=x") against the registry. */
export function parseHash(hash: string, defs: readonly ViewDef[] = views): Route {
  const raw = hash.replace(/^#\/?/, "");
  const [path = "", qs = ""] = raw.split("?");
  const query = new URLSearchParams(qs);
  for (const def of defs) {
    const params = matchPattern(def.pattern, path);
    if (params) return { view: def.id, path, params, query };
  }
  return { view: null, path, params: {}, query };
}

/** Builds "#/p/whenubuy?topic=x" from a pattern and params. */
export function href(pattern: string, params: Params = {}, query?: Record<string, string | undefined>): string {
  const path = pattern
    .split("/")
    .map((seg) => (seg.startsWith(":") ? encodeURIComponent(params[seg.slice(1)] ?? "") : seg))
    .join("/");
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(query ?? {})) if (v !== undefined) q.set(k, v);
  const qs = q.toString();
  return `#/${path}${qs ? `?${qs}` : ""}`;
}

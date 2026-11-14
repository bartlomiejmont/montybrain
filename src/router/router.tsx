import { useEffect, useState } from "react";

import { href, parseHash, type Params, type Route } from "./registry";

/** Default route when the hash is empty or unknown. */
export const HOME = "#/dzis";

export function useRoute(): Route {
  const [hash, setHash] = useState(() => window.location.hash || HOME);
  useEffect(() => {
    const onChange = () => setHash(window.location.hash || HOME);
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return parseHash(hash);
}

export function navigate(to: string): void {
  if (window.location.hash !== to) window.location.hash = to;
}

// Typed links for the built-in views.
export const links = {
  today: () => href("dzis"),
  inbox: () => href("skrzynka"),
  project: (project: string, topic?: string) => href("p/:project", { project } satisfies Params, { temat: topic }),
  /** „Mapa", optionally focused on a topic (`projekt/slug`) or a task (`^id`). */
  map: (focus?: string) => href("mapa", {}, { focus }),
};

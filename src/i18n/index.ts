import { useSyncExternalStore } from "react";

import { en } from "./en";
import { backend } from "./backend";

export type Language = "en" | "pl";
export const LANGUAGE_KEY = "montybrain.language";
export const normalizeLanguage = (value: unknown): Language => value === "pl" ? "pl" : "en";

function storedLanguage(): Language {
  try { return normalizeLanguage(localStorage.getItem(LANGUAGE_KEY)); }
  catch { return "en"; }
}
let language: Language = storedLanguage();
const listeners = new Set<() => void>();
export const getLanguage = (): Language => language;
export const getLocale = (): string => language === "pl" ? "pl-PL" : "en-GB";

export function setLanguage(value: Language) {
  const next = normalizeLanguage(value);
  try { localStorage.setItem(LANGUAGE_KEY, next); } catch { /* Keep the in-memory preference. */ }
  if (typeof document !== "undefined") {
    document.documentElement.lang = next;
    document.title = document.body.classList.contains("quick-body") ? (next === "pl" ? "Szybkie zadanie" : "Quick task") : "MontyBrain";
  }
  if (language === next) return;
  language = next;
  listeners.forEach((listener) => listener());
}

export function useLanguage(): Language {
  return useSyncExternalStore(
    (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    getLanguage,
    () => "en",
  );
}

/** Interpolate only catalog placeholders, keeping arbitrary user text intact. */
export function t(message: string, ...values: unknown[]): string {
  const template = language === "en" ? en[message] ?? backend[message] ?? message : message;
  return template.replace(/\{(\d+)\}/g, (match, index: string) => Number(index) < values.length ? String(values[Number(index)] ?? "") : match);
}

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key === LANGUAGE_KEY || event.key === null) setLanguage(storedLanguage());
  });
  setLanguage(language);
}

const escapePattern = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const messagePatterns = Object.entries(backend)
  .filter(([source]) => /\{\d+\}/.test(source))
  .sort(([a], [b]) => b.replace(/\{\d+\}/g, "").length - a.replace(/\{\d+\}/g, "").length)
  .map(([source, translated]) => ({
    source,
    translated,
    pattern: new RegExp("^" + source.split(/(\{\d+\})/).map((part) => /^\{\d+\}$/.test(part) ? "([\\s\\S]*?)" : escapePattern(part)).join("") + "$"),
  }));

const dateWords: Readonly<Record<string, string>> = {
  "dziś": "today", "jutro": "tomorrow", "pojutrze": "day after tomorrow", "wczoraj": "yesterday",
  pn: "Mon", wt: "Tue", "śr": "Wed", sr: "Wed", cz: "Thu", pt: "Fri", sb: "Sat", nd: "Sun",
  sty: "Jan", lut: "Feb", mar: "Mar", kwi: "Apr", maj: "May", cze: "Jun", lip: "Jul", sie: "Aug", wrz: "Sep", "paź": "Oct", lis: "Nov", gru: "Dec",
};
const dateMessage = (raw: string) => raw.replace(/\p{L}+/gu, (word) => dateWords[word] ?? word);

/** Translate presentation fields from Rust, keeping captured user text intact. */
export function message(raw: string): string {
  if (language === "pl") return raw;
  const exact = backend[raw] ?? en[raw];
  if (exact) return exact;
  if (/^(dziś|jutro|pojutrze|wczoraj|(?:pn|wt|śr|sr|cz|pt|sb|nd) \d)(?:\b| )/u.test(raw)) return dateMessage(raw);
  // Compound command readings have separate machine-generated clauses. A
  // separator inside a quoted task or topic title is part of user content.
  const clauses = raw.match(/(?:„[^”]*”|[^·„])+/g);
  if (clauses && clauses.length > 1 && clauses.join("·") === raw) {
    return clauses.map((part) => {
      const trimmed = part.trim();
      return part.replace(trimmed, message(trimmed));
    }).join("·");
  }
  for (const { source, translated, pattern } of messagePatterns) {
    const match = pattern.exec(raw);
    if (!match) continue;
    const values = match.slice(1);
    if (source === "Dokończ komendę: {0}" || source === "Zmieni {0}" || source === "Przeniesie {0} do: {1}") values[0] = message(values[0] ?? "");
    if (source === "priorytet {0}") values[0] = message(values[0] ?? "");
    if (source === "termin {0}") values[0] = dateMessage(values[0] ?? "");
    if (source === "„{0}”: {1}") values[1] = message(values[1] ?? "");
    return translated.replace(/\{(\d+)\}/g, (placeholder, index: string) => values[Number(index)] ?? placeholder);
  }
  return raw;
}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createMockApi } from "../api/mock";
import { formatDue, formatLongDate, shortDate } from "../lib/dates";
import { TASK_STATUS_LABEL, TOPIC_STATUS_LABEL } from "../lib/labels";
import { navTargets } from "../lib/palette";
import { plural as words } from "../lib/text";
import { plural } from "../lib/updates";
import { links } from "../router/router";
import { parseHash } from "../router/registry";
import "../views";
import { en } from "./en";
import { backend } from "./backend";
import { getLanguage, getLocale, message, normalizeLanguage, setLanguage, t } from ".";

beforeEach(() => setLanguage("en"));
afterEach(() => { setLanguage("pl"); vi.unstubAllGlobals(); });

describe("application languages", () => {
  it("defaults to English for missing, invalid and system-language values", async () => {
    expect([null, undefined, "de", "pl-PL", "en"].map(normalizeLanguage)).toEqual(["en", "en", "en", "en", "en"]);
    expect(normalizeLanguage("pl")).toBe("pl");
    expect((await createMockApi({ latencyMs: 0 }).getSettings()).language).toBe("en");
    vi.resetModules();
    const fresh = await import(".");
    expect(fresh.getLanguage()).toBe("en");
  });

  it("persists the selection and reads it on the next start", async () => {
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) });
    setLanguage("pl");
    expect(storage.get("montybrain.language")).toBe("pl");
    vi.resetModules();
    expect((await import(".")).getLanguage()).toBe("pl");
  });

  it("keeps working if storage is unavailable", () => {
    vi.stubGlobal("localStorage", { setItem: () => { throw new Error("blocked"); } });
    expect(() => setLanguage("pl")).not.toThrow();
    expect(t("Zapisz")).toBe("Zapisz");
    setLanguage("en");
    expect(t("Zapisz")).toBe("Save");
  });

  it("switches labels and locale repeatedly without stale module constants", () => {
    expect(TASK_STATUS_LABEL.doing).toBe("In progress");
    expect(TOPIC_STATUS_LABEL.waiting).toBe("Waiting");
    expect(getLocale()).toBe("en-GB");
    setLanguage("pl");
    expect(TASK_STATUS_LABEL.doing).toBe("W toku");
    expect(TOPIC_STATUS_LABEL.waiting).toBe("Czeka");
    expect(getLocale()).toBe("pl-PL");
    setLanguage("en");
    expect(TASK_STATUS_LABEL.doing).toBe("In progress");
    expect(getLanguage()).toBe("en");
  });

  it("formats dates and both kinds of plurals in the selected language", () => {
    expect(formatDue("2026-11-17T14:00", "2026-11-16")).toBe("tomorrow 14:00");
    expect(shortDate("2026-05-20", "2026-11-16")).toBe("20 May");
    expect(formatLongDate("2026-11-16")).toContain("Monday");
    expect(plural(22, t("zadanie"), t("zadania"), t("zadań"))).toBe("22 tasks");
    expect(words(2, t("temat"), t("tematy"), t("tematów"))).toBe("topics");
    setLanguage("pl");
    expect(formatDue("2026-11-17T14:00", "2026-11-16")).toBe("jutro 14:00");
    expect(shortDate("2026-05-20", "2026-11-16")).toBe("20 maj");
    expect(plural(22, t("zadanie"), t("zadania"), t("zadań"))).toBe("22 zadania");
  });

  it("does not translate user content or recursively interpolate values", () => {
    const title = 'Dziś · {1} $& <script>żółw</script>';
    expect(t("Temat {0}", title)).toBe(`Topic ${title}`);
    expect(t("My custom text")).toBe("My custom text");
    expect(message(`Nowe zadanie „${title}” w: Skrzynka`)).toBe(`New task “${title}” in: Inbox`);
    expect(message("Nie ma zadania ^abc123.")).toBe("Task ^abc123 not found.");
    expect(message("termin jutro (2026-11-17) · priorytet wysoki")).toBe("due tomorrow (2026-11-17) · high priority");
  });

  it("keeps routes and native identifiers stable while localizing navigation", async () => {
    const snap = (await createMockApi({ latencyMs: 0 }).getState()).snapshot;
    const english = navTargets(snap);
    expect(english.find((item) => item.group === "nav" && item.label === "Inbox")).toMatchObject({ to: "#/skrzynka" });
    expect(links.inbox()).toBe("#/skrzynka");
    expect(parseHash(links.inbox()).view).toBe("inbox");
    setLanguage("pl");
    expect(navTargets(snap).map((item) => item.key)).toEqual(english.map((item) => item.key));
    expect(links.inbox()).toBe("#/skrzynka");
    expect(parseHash(links.inbox()).view).toBe("inbox");
  });

  it("emits settings changes without modifying any vault content", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const before = await api.getState();
    const changed = vi.fn();
    const off = await api.onSettingsChange(changed);
    await api.updateSettings({ language: "pl" });
    await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(expect.objectContaining({ language: "pl" })));
    expect((await api.getState()).snapshot).toEqual(before.snapshot);
    await api.updateSettings({ reminders: false });
    expect((await api.getSettings()).language).toBe("pl");
    off();
  });

  it("provides nonempty translations with the same interpolation placeholders", () => {
    const placeholders = (value: string) => [...new Set(value.match(/\{\d+\}/g) ?? [])].sort();
    for (const [source, translated] of Object.entries({ ...en, ...backend })) {
      expect(translated.trim(), source).not.toBe("");
      expect(placeholders(translated), source).toEqual(placeholders(source));
    }
  });
});

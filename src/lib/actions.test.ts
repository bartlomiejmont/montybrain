import { describe, expect, it } from "vitest";

import type { ProviderInfo, Recipe } from "../api/types";
import { buildRequest, formatElapsed, formatRunTime, initialValues, parseLocal, pickProvider, runElapsedMs, validateParams } from "./actions";

const zadania: Recipe = {
  id: "zadania",
  title: "Zadania z notatek",
  description: "",
  skill: "x",
  modes: ["background", "terminal"],
  background_writes: true,
  proposes: false,
  needs_repos: false,
  params: [
    { key: "text", label: "Notatki", kind: "text", required: true, placeholder: null, default: null },
    { key: "project", label: "Projekt", kind: "project", required: false, placeholder: null, default: null },
  ],
};
const raport: Recipe = {
  ...zadania,
  id: "raport",
  params: [
    { key: "period", label: "Okres", kind: "period", required: true, placeholder: null, default: "dzis" },
    { key: "project", label: "Projekt", kind: "project", required: false, placeholder: null, default: null },
  ],
};
const prov = (id: "claude" | "codex", available: boolean): ProviderInfo => ({ id, name: id, available, path: null, version: null, problem: null, auth: { status: "unknown", method: null, plan: null, problem: null } });

describe("params", () => {
  it("initial values use defaults", () => {
    expect(initialValues(zadania)).toEqual({ text: "", project: "" });
    expect(initialValues(raport)).toEqual({ period: "dzis", project: "" });
  });
  it("requires non-blank text", () => {
    expect(validateParams(zadania, { text: "   \n", project: "" })).toHaveProperty("text");
    expect(validateParams(zadania, { text: "a", project: "" })).toEqual({});
  });
  it("validates period and project slug", () => {
    expect(validateParams(raport, { period: "rok", project: "" })).toHaveProperty("period");
    expect(validateParams(raport, { period: "tydzien", project: "nie-ma" }, ["a"])).toHaveProperty("project");
    expect(validateParams(raport, { period: "tydzien", project: "a" }, ["a"])).toEqual({});
  });
  it("builds a request with trimmed, known params only", () => {
    expect(buildRequest(zadania, "codex", "background", { text: "  kup mleko \n", project: "", extra: "x" })).toEqual({
      recipe: "zadania",
      provider: "codex",
      mode: "background",
      params: { text: "kup mleko", project: "" },
    });
  });
});

describe("provider", () => {
  it("keeps stored when available, falls back otherwise", () => {
    expect(pickProvider([prov("claude", true), prov("codex", true)], "codex")).toBe("codex");
    expect(pickProvider([prov("claude", true), prov("codex", false)], "codex")).toBe("claude");
    expect(pickProvider([prov("claude", false)], null)).toBeNull();
    expect(pickProvider([], "claude")).toBeNull();
  });
});

describe("time", () => {
  it("formats elapsed", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(7_900)).toBe("0:07");
    expect(formatElapsed(65_000)).toBe("1:05");
    expect(formatElapsed(3_723_000)).toBe("1:02:03");
    expect(formatElapsed(-5)).toBe("0:00");
    expect(formatElapsed(NaN)).toBe("0:00");
  });
  it("computes run elapsed", () => {
    const start = parseLocal("2026-11-13T10:00:00");
    expect(runElapsedMs("2026-11-13T10:00:00", null, start + 12_000)).toBe(12_000);
    expect(runElapsedMs("2026-11-13T10:00:00", "2026-11-13T10:01:05", 0)).toBe(65_000);
  });
  it("formats run time", () => {
    const now = new Date(2026, 10, 13, 15, 0);
    expect(formatRunTime("2026-11-13T14:05:09", now)).toBe("14:05");
    expect(formatRunTime("2026-11-12T09:03:00", now)).toBe("12 lis 09:03");
    expect(formatRunTime("śmieci", now)).toBe("śmieci");
  });
});

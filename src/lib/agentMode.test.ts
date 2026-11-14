import { describe, expect, it } from "vitest";

import { agentEntries, agentReach, contextLabel, planAsk } from "./agentMode";

describe("Skrzynka agent mode", () => {
  it("one entry with MontyBrain only asks for a command; everything else for proposals", () => {
    expect(planAsk("gdzie to wrzucić?", 1, "montybrain")).toEqual({ mode: "command", text: "gdzie to wrzucić?" });
    expect(planAsk("gdzie to wrzucić?", 3, "montybrain").mode).toBe("proposals");
    // With the user's own tools the answer comes back through a file: proposals.
    expect(planAsk("sprawdź na Slacku", 1, "mine").mode).toBe("proposals");
    // The verbs of the `?` commands keep their meaning; a leading `?` is dropped.
    expect(planAsk("?rozpisz dokładnie", 1, "montybrain")).toEqual({ mode: "proposals", text: "Rozpisz te wpisy na konkretne kroki. dokładnie" });
    expect(planAsk("Dopasuj", 0, "montybrain").text).toBe("Zaproponuj miejsca dla tych wpisów.");
  });

  it("the context: selected entries, else the focused one, else the whole inbox", () => {
    expect(agentEntries(["a", "b"], "c")).toEqual(["a", "b"]);
    expect(agentEntries([], "c")).toEqual(["c"]);
    expect(agentEntries([], null)).toEqual([]);
    expect(contextLabel(3, null, 9)).toBe("zaznaczone wpisy (3)");
    expect(contextLabel(0, "Faktura", 9)).toBe("wpis „Faktura”");
    expect(contextLabel(0, null, 12)).toBe("całą Skrzynkę (12 wpisów)");
    expect(contextLabel(0, null, 3)).toBe("całą Skrzynkę (3 wpisy)");
  });

  it("states plainly what each choice can and cannot reach", () => {
    const mb = agentReach("montybrain", "claude", "całą Skrzynkę (3 wpisy)");
    expect(mb.can).toContain("tylko do odczytu");
    expect(mb.cannot).toContain("Slacka");
    expect(mb.cannot).toContain("sterowania komputerem");
    const claude = agentReach("mine", "claude", "x");
    expect(claude.can).toContain("w aplikacji");
    expect(claude.returns).toContain("Terminalu");
    // Claude Code in a terminal has no computer use: said so, not implied.
    expect(claude.cannot).toContain("Claude Code (CLI) nie ma sterowania komputerem");
    const codex = agentReach("mine", "codex", "x");
    expect(codex.can).toContain("jeśli masz je włączone");
    expect(codex.cannot).toContain("tylko do odczytu");
    for (const r of [mb, claude, codex]) expect(`${r.can} ${r.cannot} ${r.returns}`).toMatch(/propozycj/i);
  });
});

import { describe, expect, it } from "vitest";
import { explainWarning } from "./warnings";

describe("explainWarning", () => {
  it("translates fixture and core parser warnings into Polish while preserving raw details", () => {
    const due = explainWarning("invalid due date `@2026-02-30`; kept as title text");
    expect(due.summary).toContain("Nieprawidłowa data terminu (@2026-02-30)");
    expect(due.raw).toBe("invalid due date `@2026-02-30`; kept as title text");

    const box = explainWarning("checkbox `[?]` is not a task status; line kept as is");
    expect(box.summary).toContain("Nieznany znacznik pola wyboru [?]");
    expect(box.summary).toContain("nie został wczytany jako zadanie");

    const fm = explainWarning("frontmatter `status`: bad value");
    expect(fm.summary).toContain("nagłówku pliku");

    const fallback = explainWarning("something completely unexpected");
    expect(fallback.summary).toContain("Nierozpoznany zapis");
    expect(fallback.raw).toBe("something completely unexpected");
  });
});
